//! A single cancellable project worker owns disk caches and the mutable analysis host.
use super::*;
use folio_diagnostics::Diagnostic;
use folio_lint::{LintConfig, lint_script};
use folio_project_model::SourceFile;
use folio_project_resolve::{
    discover,
    io::{LoadCache, LoadedSourceInput, WatchPlan},
    resolve,
};
use std::sync::mpsc::{self, Receiver, Sender};

pub(super) struct Job {
    pub generation: u64,
    pub refresh_disk: bool,
    pub overlays: BTreeMap<PathBuf, Arc<str>>,
}

pub(super) struct Prepared {
    pub loaded: Arc<LoadedProject>,
    pub metadata: Arc<Metadata>,
    pub view: Arc<ProjectAnalysisView>,
    pub disk_sources: Vec<LoadedSourceInput>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) enum Event {
    Watches(u64, WatchPlan),
    Progress(u64, &'static str, String, usize, usize),
    Finished(u64, Result<Option<Prepared>, String>),
}

pub(super) struct Loader {
    jobs: Sender<Job>,
    pub events: Receiver<Event>,
}

impl Loader {
    pub fn new(
        cwd: PathBuf,
        manifest: Option<PathBuf>,
        home: FolioHome,
        generation: Arc<AtomicU64>,
    ) -> Self {
        let (jobs, incoming) = mpsc::channel::<Job>();
        let (events, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("folio-project".into())
            .spawn(move || {
                let mut worker = Worker {
                    cwd,
                    manifest,
                    home,
                    generation,
                    events,
                    cache: LoadCache::default(),
                    disk: None,
                    project: ProjectAnalysis::new(),
                };
                while let Ok(job) = incoming.recv() {
                    let revision = job.generation;
                    let result = worker.prepare(job);
                    if worker
                        .events
                        .send(Event::Finished(revision, result))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("start project worker");
        Self {
            jobs,
            events: results,
        }
    }

    pub fn submit(&self, job: Job) -> Result<(), LspError> {
        self.jobs
            .send(job)
            .map_err(|_| LspError::Protocol("project worker stopped".into()))
    }
}

struct Worker {
    cwd: PathBuf,
    manifest: Option<PathBuf>,
    home: FolioHome,
    generation: Arc<AtomicU64>,
    events: Sender<Event>,
    cache: LoadCache,
    disk: Option<(LoadedProject, Arc<Metadata>)>,
    project: ProjectAnalysis,
}

impl Worker {
    fn prepare(&mut self, job: Job) -> Result<Option<Prepared>, String> {
        let started = Instant::now();
        let current = Arc::clone(&self.generation);
        let obsolete = || current.load(Ordering::SeqCst) != job.generation;
        if obsolete() {
            return Ok(None);
        }
        if job.refresh_disk || self.disk.is_none() {
            // A failed or cancelled refresh must never make later buffer edits use old disk inputs.
            self.disk = None;
            let plan = folio_project_resolve::io::watch_plan_with_home(
                &self.cwd,
                self.manifest.as_deref(),
                &self.home,
            );
            let _ = self.events.send(Event::Watches(job.generation, plan));
            let events = &self.events;
            let loaded = folio_project_resolve::io::load_cached_with_home(
                &self.cwd,
                self.manifest.as_deref(),
                &self.home,
                &mut self.cache,
                &mut |completed, total, name| {
                    let _ = events.send(Event::Progress(
                        job.generation,
                        "dependencies",
                        if name.is_empty() {
                            "Dependencies loaded".into()
                        } else {
                            format!("Loading dependencies: {name}")
                        },
                        completed,
                        total,
                    ));
                    !obsolete()
                },
            );
            if obsolete() {
                return Ok(None);
            }
            let loaded = loaded.map_err(|error| error.to_string())?;
            let _ = self
                .events
                .send(Event::Watches(job.generation, loaded.watch_plan.clone()));
            let metadata =
                resolve(&loaded.root, &loaded.dependencies).map_err(|error| error.to_string())?;
            self.disk = Some((loaded, Arc::new(metadata)));
        }
        if obsolete() {
            return Ok(None);
        }
        let load_us = started.elapsed().as_micros();
        let projection_started = Instant::now();
        let (disk, metadata) = self.disk.as_ref().expect("loaded project");
        let disk_sources = disk.source_inputs.clone();
        let mut loaded = disk.clone();
        let mut metadata = Arc::clone(metadata);
        let disk_paths = disk_sources
            .iter()
            .map(|source| source.canonical_path.clone())
            .collect();
        Self::add_unsaved_sources(
            &self.cwd,
            self.manifest.as_deref(),
            &job.overlays,
            &mut loaded,
            &disk_paths,
        )
        .map_err(|error| error.to_string())?;
        let dependency_changed = overlays::apply_dependency_overlays(&mut loaded, &job.overlays)
            .map_err(|error| error.to_string())?;
        if loaded.source_inputs.len() != disk_sources.len() || dependency_changed {
            metadata = Arc::new(
                resolve(&loaded.root, &loaded.dependencies).map_err(|error| error.to_string())?,
            );
        }
        for source in &mut loaded.source_inputs {
            if let Some(text) = job.overlays.get(&source.canonical_path) {
                source.text = Arc::clone(text);
            }
        }
        if obsolete() {
            return Ok(None);
        }
        let files = loaded.source_inputs.len();
        let _ = self.events.send(Event::Progress(
            job.generation,
            "analysis",
            "Analyzing project sources".into(),
            0,
            files,
        ));
        let view = Arc::new(
            self.project
                .sync_project(&loaded, &metadata)
                .map_err(|error| error.to_string())?,
        );
        let projection_us = projection_started.elapsed().as_micros();
        let analysis_started = Instant::now();
        if view.analysis.try_warm_semantics(obsolete).is_err() {
            return Ok(None);
        }
        let analysis_us = analysis_started.elapsed().as_micros();
        let diagnostics_started = Instant::now();
        let mut diagnostics = view.diagnostics();
        let lint = LintConfig::from_rules(&loaded.root.manifest.lint_rules)
            .map_err(|error| error.to_string())?;
        for (&file, source) in &view.sources {
            if obsolete() {
                return Ok(None);
            }
            if source.package_key == loaded.root_key
                && let Some(script) = view.analysis.hir(file)
            {
                diagnostics.extend(lint_script(&script, &lint));
            }
        }
        let _ = self.events.send(Event::Progress(
            job.generation,
            "analysis",
            "Project analysis complete".into(),
            files,
            files,
        ));
        tracing::debug!(
            generation = job.generation,
            files,
            load_us,
            projection_us,
            analysis_us,
            diagnostics_us = diagnostics_started.elapsed().as_micros(),
            elapsed_us = started.elapsed().as_micros(),
            "prepared LSP project snapshot"
        );
        Ok(Some(Prepared {
            loaded: Arc::new(loaded),
            metadata,
            view,
            disk_sources,
            diagnostics,
        }))
    }
    /// Projects new open files through the same root manifest and resolver graph as disk files.
    fn add_unsaved_sources(
        cwd: &Path,
        manifest_path: Option<&Path>,
        overlays: &BTreeMap<PathBuf, Arc<str>>,
        loaded: &mut folio_project_resolve::LoadedProject,
        disk_paths: &BTreeSet<PathBuf>,
    ) -> Result<(), LspError> {
        let manifest_path =
            discover(cwd, manifest_path).map_err(|error| LspError::Project(error.to_string()))?;
        let base = manifest_path
            .parent()
            .ok_or_else(|| LspError::Project("manifest has no parent".into()))?;
        let manifest = loaded.root.manifest.clone();
        for (path, text) in overlays.iter() {
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
                text: Arc::clone(text),
            });
        }
        Ok(())
    }
}
