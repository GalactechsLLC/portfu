use std::collections::{HashMap, HashSet};
use std::io::{Error, ErrorKind};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use async_trait::async_trait;
use http::Extensions;
use log::{error, info, trace};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{channel, Sender};
use tokio::sync::RwLock;
use tokio::task::{spawn_blocking};
use tokio::time::Instant;
use uuid::Uuid;
use crate::files::{FileLoader, get_mime_type, read_directory};
use crate::server::Server;
use crate::service::{Service, ServiceBuilder, ServiceGroup};
use crate::signal::await_termination;
use crate::task::{TaskFn};

pub struct NpmSinglePageApp {
    managed_files: RwLock<HashSet<Uuid>>,
    pub src_directory: PathBuf,
    pub output_directory: PathBuf,
    pub build_command: String, // (npm|yarn) run {build_command}
    pub editable: bool,

}
impl NpmSinglePageApp {
    pub fn new(src_directory: PathBuf, output_directory: PathBuf, build_command: String) -> Self {
        Self {
            managed_files: Default::default(),
            src_directory,
            output_directory,
            build_command,
            editable: true,
        }
    }
}
#[async_trait]
impl TaskFn for NpmSinglePageApp {
    fn name(&self) -> &str {
        "Npm Watcher Service"
    }

    async fn run(&self, state: Arc<RwLock<Extensions>>) -> Result<(), Error> {
        let (tx, mut rx) = channel(64);
        struct EventHandler {
            tx: Sender<notify::Result<Event>>,
            runtime: Runtime,
        }
        info!("Starting Filewatcher on {:?}", &self.src_directory);
        impl notify::EventHandler for EventHandler {
            fn handle_event(&mut self, event: notify::Result<Event>) {
                let runtime = &self.runtime;
                let tx = &self.tx;
                runtime.block_on(async move {
                    if let Err(e) = tx.send(event).await {
                        error!("Failed to send Notify Event: {e:?}");
                    }
                });
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create async context");
        let mut watcher =
            RecommendedWatcher::new(EventHandler { tx, runtime }, Config::default()).map_err(|e| {
                Error::new(ErrorKind::Other, format!("Failed to spawn watcher: {e:?}"))
            })?;
        watcher.watch(&self.src_directory, RecursiveMode::Recursive).map_err(|e| {
            Error::new(ErrorKind::Other, format!("Failed to add src directory to watcher: {e:?}"))
        })?;
        match state.read().await.get::<Arc<Server>>().cloned() {
            None => {
                Err(Error::new(ErrorKind::NotFound, "Failed to Find Server Object, Cannot do Dynamic Loading."))
            }
            Some(server) => {
                let mut last_update = Instant::now();
                loop {
                    tokio::select! {
                        _ = await_termination() => {
                            break;
                        }
                        res = rx.recv() => {
                            match res {
                                Some(Ok(event)) => {
                                    match event.kind {
                                        EventKind::Remove(_) | EventKind::Create(_) | EventKind::Modify(_) => {
                                            if Instant::now().duration_since(last_update) < Duration::from_secs(5) {
                                                trace!("Ignoring Event: {:?} - Too Soon", event.kind);
                                                continue;
                                            }
                                            //Consume All Events in case of Multiple File Changes
                                            info!("Got Event {:?} Rebuilding", &event.kind);
                                            while !rx.is_empty() {
                                                let _ = rx.recv().await;
                                            }
                                            let src_directory = self.src_directory.clone();
                                            let output_directory = self.output_directory.clone();
                                            let editable = self.editable;
                                            match spawn_blocking(|| -> Result<(), Error> {
                                                run_build(src_directory)
                                            }).await {
                                                Ok(Ok(_)) => {
                                                    info!("Rebuild Finished. Reloading Services");
                                                    let current_services = self.managed_files.read().await.clone();
                                                    let mut files = HashMap::new();
                                                    if let Err(e) = read_directory(&output_directory, output_directory.clone(), &mut files) {
                                                        log::error!("Error Loading files: {e:?}");
                                                    }
                                                    let services: Vec<Service> = files
                                                        .into_iter()
                                                        .map(|(name, path)| {
                                                            let mime = get_mime_type(&name);
                                                            ServiceBuilder::new(&name)
                                                                .name(&name)
                                                                .handler(Arc::new(FileLoader {
                                                                    name,
                                                                    mime,
                                                                    path,
                                                                    editable,
                                                                    cache_threshold: 65536,
                                                                    cache_status: AtomicBool::default(),
                                                                    cached_value: Arc::new(RwLock::new(Vec::with_capacity(0))),
                                                                }))
                                                                .build()
                                                        })
                                                        .collect();
                                                    server.registry.write().await.services.retain(|s| {
                                                        !current_services.contains(&s.uuid)
                                                    });
                                                    for service in services {
                                                        server.registry.write().await.register(service);
                                                    }
                                                    info!("Finished Reloading Services");
                                                }
                                                Ok(Err(e)) => {
                                                    error!("Failed to run build command: {e:?}")
                                                }
                                                Err(e) => {
                                                    error!("Failed to join build command: {e:?}")
                                                }
                                            }
                                            tokio::time::sleep(Duration::from_secs(1)).await;
                                            //Clear Events that Happened During Build
                                            info!("Clearing Evnet Queue From Build");
                                            while !rx.is_empty() {
                                                let _ = rx.recv().await;
                                            }
                                            last_update = Instant::now();
                                        }
                                        _ => {}
                                    }
                                }
                                Some(Err(error)) => error!("Watcher Error: {error:?}"),
                                None => {
                                    error!("Watcher Notifier Channel is Closing.")
                                },
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }
}
impl From<NpmSinglePageApp> for ServiceGroup {
    fn from(mut slf: NpmSinglePageApp) -> ServiceGroup {
        let mut files = HashMap::new();
        info!("Searching for Node Project at: {:?}", &slf.src_directory);
        let mut build = true;
        if !slf.src_directory.exists() {
            if let Err(e) = std::fs::create_dir(&slf.src_directory) {
                log::error!("Error Creating Src Directory: {e:?}");
                build = false;
            }
        }
        if !slf.output_directory.exists() {
            if let Err(e) = std::fs::create_dir(&slf.output_directory) {
                log::error!("Error Creating Output Directory: {e:?}");
                build = false;
            }
        }
        let mut package_file = slf.src_directory.clone();
        package_file.push("package.json");
        if !package_file.exists() {
            log::error!("No package.json file found in src directory, not building");
            build = false;
        }
        if build {
            let _ = run_build(&slf.src_directory);
        }
        if let Err(e) = read_directory(&slf.output_directory, slf.output_directory.clone(), &mut files) {
            log::error!("Error Loading files: {e:?}");
        }
        let editable = slf.editable;
        let services: Vec<Service> = files
            .into_iter()
            .map(|(name, path)| {
                let mime = get_mime_type(&name);
                ServiceBuilder::new(&name)
                    .name(&name)
                    .handler(Arc::new(FileLoader {
                        name,
                        mime,
                        path,
                        editable,
                        cache_threshold: 65536,
                        cache_status: AtomicBool::default(),
                        cached_value: Arc::new(RwLock::new(Vec::with_capacity(0))),
                    }))
                    .build()
            })
            .collect();
        slf.managed_files = RwLock::new(HashSet::from_iter(services.iter().map(|v| v.uuid)));
        ServiceGroup {
            filters: vec![],
            wrappers: vec![],
            tasks: vec![Arc::new(slf)],
            services,
            shared_state: Default::default(),
        }
    }
}

pub fn run_build<P: AsRef<std::path::Path>>(src_directory: P) -> Result<(), Error> {
    let mut npm_exists = false;
    match Command::new("npm").spawn() {
        Ok(mut c) => {
            let _ = c.kill();
            npm_exists = true
        },
        Err(e) => {
            if let ErrorKind::NotFound = e.kind() {
                npm_exists = false;
            } else {
                info!("Failed to Check for npm: {e:?}");
            }
        },
    }
    let mut yarn_exists = false;
    if !npm_exists {
        //Check Yarn only if NPM Doesnt exist
        match Command::new("yarn").spawn() {
            Ok(mut c) => {
                let _ = c.kill();
                yarn_exists = true
            },
            Err(e) => {
                if let ErrorKind::NotFound = e.kind() {
                    yarn_exists = false;
                } else {
                    info!("Failed to Check for yarn: {e:?}");
                }
            },
        }
    }
    if !npm_exists && !yarn_exists {
        error!("Failed to Find 'npm' or 'yarn', install either to manage nodejs projects");
    } else {
        let path = src_directory.as_ref();
        info!("Building Node Project at: {:?}", path);
        let mut cmd = Command::new(if npm_exists { "npm" } else { "yarn"});
        cmd.current_dir(path);
        cmd.arg("run").arg("build").stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = cmd.spawn()?;
        let output = child.wait_with_output()?;
        if output.status.success() {
            info!("Build succeeded");
        } else {
            error!("Failed to Run Build: {} \n {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
        }
    }
    Ok(())
}