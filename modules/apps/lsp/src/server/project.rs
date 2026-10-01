//! Project reloads, document overlays, and dependency watch registration.
use super::*;
use crate::protocol::path_to_uri;
use folio_lint::LintConfig;
use folio_project_model::SourceFile;
use folio_project_resolve::{discover, io::LoadedSourceInput, resolve};

impl Server {
    pub(super) fn reload_report(&mut self, refresh_disk: bool) -> Result<(), LspError> {
        if let Err(error) = self.reload(refresh_disk) {
            tracing::error!(%error, "LSP project analysis unavailable");
            self.clear_view()?;
            self.disk_project = None;
            let message = error.to_string();
            if self.last_error.as_ref() != Some(&message) {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":message}}),
                )?;
                self.last_error = Some(message);
            }
        } else {
            self.last_error = None;
        }
        Ok(())
    }

    /// Buffer lifecycle alone does not change semantic inputs or invalidate requests.
    pub(super) fn update_document(&mut self, path: &Path) -> Result<(), LspError> {
        let unchanged = self
            .paths
            .get(path)
            .and_then(|file| self.view.as_ref()?.analysis.text(*file))
            .is_some_and(|text| Some(text) == self.documents.text(&path.to_path_buf()));
        if unchanged {
            let uri = path_to_uri(path);
            if let Some(diagnostics) = self.diagnostics.get(&uri) {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":self.documents.version(&path.to_path_buf()),"diagnostics":diagnostics}}),
                )?;
            }
            tracing::debug!(path = %path.display(), "reused unchanged LSP project view");
            return Ok(());
        }
        self.reload_report(false)
    }

    /// Re-read the closed file so discarding a buffer restores even unwatched disk edits.
    pub(super) fn close_document(&mut self, path: &Path) -> Result<(), LspError> {
        if let Some((loaded, _)) = self.disk_project.as_mut()
            && let Some(source) = loaded
                .source_inputs
                .iter_mut()
                .find(|source| source.canonical_path == path)
            && let Ok(text) = std::fs::read_to_string(path)
        {
            source.text = Arc::from(text);
            self.documents
                .disk_update(path.to_path_buf(), Arc::clone(&source.text));
            return self.update_document(path);
        }
        // New unsaved files and removed files require rebuilding provider selection.
        self.reload_report(true)
    }

    fn reload(&mut self, refresh_disk: bool) -> Result<(), LspError> {
        if !self.initialized || self.shutdown {
            return Ok(());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();
        if refresh_disk || self.disk_project.is_none() {
            // Register candidate paths before reading carriers so missing or ambiguous
            // inputs can recover when files or intermediate directories change.
            self.register_file_watches(generation)?;
            match folio_project_resolve::io::load_and_resolve_with_home(
                &self.cwd,
                self.manifest_path.as_deref(),
                &self.home,
            ) {
                Ok(project) => self.disk_project = Some(project),
                Err(error) => {
                    tracing::error!(%error, "LSP project reload failed");
                    return Err(LspError::Project(error.to_string()));
                }
            }
        }
        let (mut loaded, mut metadata) = self
            .disk_project
            .as_ref()
            .expect("loaded disk project")
            .clone();
        if refresh_disk {
            self.register_file_watches(generation)?;
        }
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "load",
            "LSP project phase complete"
        );
        let started = Instant::now();
        let mut disk_paths = BTreeSet::new();
        for source in &loaded.source_inputs {
            disk_paths.insert(source.canonical_path.clone());
            self.documents
                .disk_update(source.canonical_path.clone(), Arc::clone(&source.text));
        }
        self.documents.retain_disk_paths(&disk_paths);
        self.add_unsaved_sources(&mut loaded, &disk_paths)?;
        if loaded.source_inputs.len() != disk_paths.len() {
            metadata = resolve(&loaded.root, &loaded.dependencies)
                .map_err(|error| LspError::Project(error.to_string()))?;
        }
        for source in &mut loaded.source_inputs {
            if let Some(text) = self.documents.text(&source.canonical_path) {
                source.text = Arc::from(text);
            }
        }
        let view = Arc::new(
            self.project
                .sync_project(&loaded, &metadata)
                .map_err(|error| LspError::Project(error.to_string()))?,
        );
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "projection",
            "LSP project phase complete"
        );
        self.metadata = Some(Arc::new(metadata));
        self.paths = view
            .sources
            .iter()
            .map(|(&file, source)| (source.canonical_path.clone(), file))
            .collect();
        tracing::info!(
            generation,
            files = self.paths.len(),
            "LSP project view updated"
        );
        let rules = loaded.root.manifest.lint_rules.clone();
        let lint =
            LintConfig::from_rules(&rules).map_err(|error| LspError::Project(error.to_string()))?;
        self.publish(view.clone(), generation, &loaded.root_key, &lint)?;
        self.view = Some(view);
        Ok(())
    }

    /// The resolver owns dependency paths, including carriers outside the editor folder.
    fn register_file_watches(&mut self, generation: u64) -> Result<(), LspError> {
        // Capability registration is allowed only after the initialized notification.
        if !self.dynamic_watches || !self.client_ready {
            return Ok(());
        }
        let plan = folio_project_resolve::io::watch_plan_with_home(
            &self.cwd,
            self.manifest_path.as_deref(),
            &self.home,
        );
        let mut patterns = BTreeSet::new();
        for directory in &plan.directories {
            patterns.insert((path_to_uri(directory), String::from("**/*")));
        }
        for file in &plan.files {
            if let (Some(parent), Some(name)) = (file.parent(), file.file_name())
                && parent.is_dir()
            {
                patterns.insert((path_to_uri(parent), name.to_string_lossy().into_owned()));
            }
        }
        let watchers = json!(
            patterns
                .into_iter()
                .map(|(base, pattern)| {
                    json!({"globPattern":{"baseUri":base,"pattern":pattern},"kind":7})
                })
                .collect::<Vec<_>>()
        );
        if self.watchers.as_ref() == Some(&watchers) {
            return Ok(());
        }
        if self.watchers.is_some() {
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","id":format!("folio/watch/unregister/{generation}"),"method":"client/unregisterCapability","params":{"unregisterations":[{"id":"folio-project-files","method":"workspace/didChangeWatchedFiles"}]}}),
            )?;
        }
        tracing::debug!(
            count = watchers.as_array().map_or(0, Vec::len),
            "registered LSP project file watches"
        );
        send(
            &self.output,
            &json!({"jsonrpc":"2.0","id":format!("folio/watch/register/{generation}"),"method":"client/registerCapability","params":{"registrations":[{"id":"folio-project-files","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":watchers}}]}}),
        )?;
        self.watchers = Some(watchers);
        Ok(())
    }

    /// Projects new open files through the same root manifest and resolver graph as disk files.
    fn add_unsaved_sources(
        &self,
        loaded: &mut folio_project_resolve::LoadedProject,
        disk_paths: &BTreeSet<PathBuf>,
    ) -> Result<(), LspError> {
        let manifest_path = discover(&self.cwd, self.manifest_path.as_deref())
            .map_err(|error| LspError::Project(error.to_string()))?;
        let base = manifest_path
            .parent()
            .ok_or_else(|| LspError::Project("manifest has no parent".into()))?;
        let manifest = loaded.root.manifest.clone();
        for (path, text) in self.documents.overlays() {
            if disk_paths.contains(path) {
                continue;
            }
            let display_path = std::fs::canonicalize(base.join(&manifest.source_path.value))
                .ok()
                .and_then(|source_root| path.strip_prefix(source_root).ok().map(Path::to_path_buf))
                .map(|relative| {
                    Path::new(&manifest.source_path.value)
                        .join(relative)
                        .to_string_lossy()
                        .replace('\\', "/")
                });
            let extension_allowed = path.extension().is_some_and(|extension| {
                manifest
                    .extensions
                    .iter()
                    .any(|item| extension.eq_ignore_ascii_case(item))
            });
            if !extension_allowed {
                continue;
            }
            let Some(display_path) = display_path else {
                continue;
            };
            let candidate = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| {
                    LspError::Project(format!("invalid script name: {}", path.display()))
                })?;
            let portable = folio_project_resolve::io::relative_portable(base, path)
                .map_err(|error| LspError::Project(error.to_string()))?;
            tracing::debug!(path = %path.display(), "added unsaved project source");
            loaded.root.source_files.push(SourceFile {
                path: portable.clone(),
                display_path: display_path.clone(),
                script_candidate: candidate.to_owned(),
            });
            loaded.source_inputs.push(LoadedSourceInput {
                package_key: loaded.root_key.clone(),
                canonical_path: path.clone(),
                display_path,
                script_candidate: candidate.to_owned(),
                text: Arc::from(text),
            });
        }
        Ok(())
    }
}
