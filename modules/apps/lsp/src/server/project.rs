//! Coalesced loading and publication of immutable project snapshots.
use super::*;
use crate::protocol::path_to_uri;

impl Server {
    /// Broad client watchers may report unrelated project JSON and generated output.
    pub(super) fn file_event_relevant(&self, uri: &Value) -> bool {
        let Some(path) = uri.as_str().and_then(uri_to_path) else {
            // A deleted subtree may no longer have a canonicalizable parent.
            // Conservatively refresh instead of leaving stale dependency inputs.
            return uri.as_str().is_some_and(|uri| uri.starts_with("file://"));
        };
        let Some(loaded) = self
            .projected_inputs
            .as_ref()
            .filter(|_| self.last_error.is_none())
        else {
            return true;
        };
        relevant_path(&path, &loaded.watch_plan)
    }
    /// Coalesce bursts while invalidating obsolete workers immediately.
    pub(super) fn reload_report(&mut self, refresh_disk: bool) -> Result<(), LspError> {
        if !self.initialized || self.shutdown {
            return Ok(());
        }
        self.advance_generation()?;
        self.reload.request(refresh_disk, Instant::now());
        self.loading = true;
        self.completions.clear();
        self.status("loading", "queued", "Loading project", None)
    }

    pub(super) fn update_document(&mut self, path: &Path) -> Result<(), LspError> {
        let unchanged = !self.loading
            && self
                .paths
                .get(path)
                .and_then(|file| self.view.as_ref()?.analysis.text(*file))
                .is_some_and(|text| Some(text) == self.documents.text(&path.to_path_buf()));
        if unchanged {
            let uri = path_to_uri(path);
            if let Some(diagnostics) = self.diagnostics.get(&uri) {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
                    "uri":uri,"version":self.documents.version(&path.to_path_buf()),"diagnostics":diagnostics}}),
                )?;
            }
            return Ok(());
        }
        self.reload_report(false)
    }

    pub(super) fn close_document(&mut self, _path: &Path) -> Result<(), LspError> {
        // The worker rereads disk, including new/deleted files and discarded dependency buffers.
        self.reload_report(true)
    }

    fn status(
        &self,
        state: &str,
        phase: &str,
        message: &str,
        progress: Option<(usize, usize)>,
    ) -> Result<(), LspError> {
        if self.status_supported {
            let mut params = json!({"state":state,"phase":phase,"message":message,"generation":self.generation.load(Ordering::SeqCst)});
            if let Some((completed, total)) = progress {
                params["completed"] = json!(completed);
                params["total"] = json!(total);
            }
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","method":"folio/status","params":params}),
            )?;
        }
        Ok(())
    }

    /// Called by the protocol event loop even when no client messages arrive.
    pub(crate) fn poll_background(&mut self) -> Result<(), LspError> {
        while let Some(event) = self
            .loader
            .as_ref()
            .and_then(|loader| loader.events.try_recv().ok())
        {
            match event {
                loading::Event::Watches(generation, plan)
                    if generation == self.generation.load(Ordering::SeqCst) && !self.shutdown =>
                {
                    self.register_file_watches(generation, &plan)?;
                }
                loading::Event::Progress(generation, phase, message, done, total)
                    if generation == self.generation.load(Ordering::SeqCst) && !self.shutdown =>
                {
                    self.status("loading", phase, &message, Some((done, total)))?;
                }
                loading::Event::Finished(generation, result) => {
                    self.reload.finish(false);
                    if generation != self.generation.load(Ordering::SeqCst) || self.shutdown {
                        continue;
                    }
                    match result {
                        Ok(Some(prepared)) => {
                            let disk_paths = prepared
                                .disk_sources
                                .iter()
                                .map(|source| source.canonical_path.clone())
                                .collect();
                            for source in prepared.disk_sources {
                                self.documents
                                    .disk_update(source.canonical_path, source.text);
                            }
                            self.documents.retain_disk_paths(&disk_paths);
                            self.paths = prepared
                                .view
                                .sources
                                .iter()
                                .map(|(&file, source)| (source.canonical_path.clone(), file))
                                .collect();
                            self.publish(
                                Arc::clone(&prepared.view),
                                generation,
                                prepared.diagnostics,
                            )?;
                            self.ide =
                                Some(folio_ide::IdeSnapshot::new(Arc::clone(&prepared.view)));
                            self.view = Some(prepared.view);
                            self.metadata = Some(prepared.metadata);
                            self.projected_inputs = Some(prepared.loaded);
                            self.loading = false;
                            self.reload.finish(true);
                            self.last_error = None;
                            tracing::info!(
                                generation,
                                files = self.paths.len(),
                                "LSP project ready"
                            );
                            self.status("ready", "ready", "Ready", None)?;
                            self.refresh_editor()?;
                        }
                        Ok(None) => {}
                        Err(message) => {
                            self.loading = false;
                            self.clear_view()?;
                            tracing::error!(generation, %message, "LSP project analysis unavailable");
                            self.status("error", "failed", &message, None)?;
                            if self.last_error.as_ref() != Some(&message) {
                                send(
                                    &self.output,
                                    &json!({"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":message}}),
                                )?;
                                self.last_error = Some(message);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !self.shutdown
            && let Some(refresh_disk) = self.reload.take_due(Instant::now())
        {
            if self.loader.is_none() {
                self.loader = Some(loading::Loader::new(
                    self.cwd.clone(),
                    self.manifest_path.clone(),
                    self.home.clone(),
                    Arc::clone(&self.generation),
                ));
            }
            let job = loading::Job {
                generation: self.generation.load(Ordering::SeqCst),
                refresh_disk,
                overlays: self
                    .documents
                    .overlays()
                    .map(|(path, text)| (path.clone(), Arc::from(text)))
                    .collect(),
            };
            self.loader.as_ref().unwrap().submit(job)?;
        }
        self.poll_deferred()?;
        Ok(())
    }

    /// Refresh optional client UI only after its backing snapshot has been published.
    pub(super) fn refresh_editor(&self) -> Result<(), LspError> {
        if !self.client_ready {
            return Ok(());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        if self.declaration_documents {
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","method":"folio/projectChanged","params":{"generation":generation}}),
            )?;
        }
        for (supported, feature) in [
            (self.semantic_tokens_refresh, "semanticTokens"),
            (self.code_lens_refresh, "codeLens"),
            (self.inlay_hint_refresh, "inlayHint"),
        ] {
            if supported {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","id":format!("folio/refresh/{feature}/{generation}"),"method":format!("workspace/{feature}/refresh")}),
                )?;
            }
        }
        Ok(())
    }

    /// The resolver owns dependency paths, including carriers outside the editor folder.
    fn register_file_watches(
        &mut self,
        generation: u64,
        plan: &folio_project_resolve::io::WatchPlan,
    ) -> Result<(), LspError> {
        // Capability registration is allowed only after the initialized notification.
        if !self.dynamic_watches || !self.client_ready {
            return Ok(());
        }
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
}

fn relevant_path(path: &Path, plan: &folio_project_resolve::io::WatchPlan) -> bool {
    if plan
        .files
        .iter()
        .any(|file| path == file || file.starts_with(path))
    {
        return true;
    }
    let source = path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("psc") || extension.eq_ignore_ascii_case("pex")
    });
    plan.directories.iter().any(|directory| {
        directory.starts_with(path)
            || (path.starts_with(directory) && (source || path.extension().is_none()))
    })
}

#[cfg(test)]
mod tests;
