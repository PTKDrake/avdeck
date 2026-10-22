//! Device management dialogs: create, edit, duplicate, delete, wipe, start with options, and the
//! snapshots of a running device. Slow work (the JVM tools, copying gigabytes) runs on a thread
//! as a `Job` the dialog polls.

use crate::app::{App, modal};
use crate::icons;
use crate::theme;
use avdeck_core::avd::{self, Avd};
use avdeck_core::images::{self, SystemImage};
use avdeck_core::launch::{self, GPU_MODES};
use avdeck_core::tool;
use avdeck_grpc::Emulator;
use avdeck_grpc::control::snapshot_filter::LoadStatus;
use avdeck_grpc::control::{SnapshotFilter, SnapshotPackage};
use eframe::egui::{self, Color32, RichText};
use std::sync::{Arc, Mutex};

/// Progress text, then the result.
type JobState<T> = (String, Option<Result<T, String>>);

/// Work on another thread: progress text while it runs, then the result.
pub struct Job<T> {
    shared: Arc<Mutex<JobState<T>>>,
}

impl<T: Send + 'static> Job<T> {
    fn spawn(
        ctx: &egui::Context,
        work: impl FnOnce(&dyn Fn(String)) -> Result<T, String> + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Mutex::new((String::new(), None)));
        let thread_shared = shared.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let progress_shared = thread_shared.clone();
            let progress_ctx = ctx.clone();
            let progress = move |text: String| {
                progress_shared.lock().unwrap().0 = text;
                progress_ctx.request_repaint();
            };
            let result = work(&progress);
            thread_shared.lock().unwrap().1 = Some(result);
            ctx.request_repaint();
        });
        Self { shared }
    }

    fn progress(&self) -> String {
        self.shared.lock().unwrap().0.clone()
    }

    /// The result once done (taken: returns it once).
    fn take(&self) -> Option<Result<T, String>> {
        self.shared.lock().unwrap().1.take()
    }
}

pub enum Dialog {
    Create(Box<Create>),
    Edit(Box<Edit>),
    Duplicate {
        avd: Avd,
        name: String,
        id: String,
        job: Option<Job<Avd>>,
        error: Option<String>,
    },
    Delete {
        avd: Avd,
        job: Option<Job<()>>,
        error: Option<String>,
    },
    Wipe(Avd),
    Start {
        avd: Avd,
        options: launch::Options,
        extra: String,
    },
    Snapshots(Box<Snapshots>),
}

pub struct Create {
    name: String,
    id: String,
    /// The id was typed, not derived from the name.
    id_edited: bool,
    devices: Vec<String>,
    devices_job: Option<Job<Vec<String>>>,
    device: String,
    installed: Vec<SystemImage>,
    available: Vec<SystemImage>,
    available_job: Option<Job<Vec<SystemImage>>>,
    install_job: Option<Job<()>>,
    image: Option<String>,
    ram_mb: u32,
    storage_gb: u32,
    job: Option<Job<String>>,
    error: Option<String>,
}

pub struct Edit {
    avd: Avd,
    name: String,
    ram_mb: u32,
    storage: String,
    cores: u32,
    gpu: String,
    keyboard: bool,
    cold_boot: bool,
    error: Option<String>,
}

pub struct Snapshots {
    avd: Avd,
    emulator: Option<Emulator>,
    list: Option<Job<Vec<(String, String)>>>,
    snapshots: Vec<(String, String)>,
    name: String,
    action: Option<Job<String>>,
    message: Option<String>,
}

impl Dialog {
    pub fn create(app: &App) -> Self {
        let installed = app
            .tools
            .sdk
            .as_ref()
            .map(|sdk| images::installed(&sdk.path))
            .unwrap_or_default();
        Self::Create(Box::new(Create {
            name: String::new(),
            id: String::new(),
            id_edited: false,
            devices: Vec::new(),
            devices_job: None,
            device: "pixel_9".to_owned(),
            image: installed.first().map(|image| image.package.clone()),
            installed,
            available: Vec::new(),
            available_job: None,
            install_job: None,
            ram_mb: 4096,
            storage_gb: 16,
            job: None,
            error: None,
        }))
    }

    pub fn edit(avd: &Avd) -> Self {
        let values = avd::values(avd);
        let number = |key: &str, default: u32| {
            values
                .get(key)
                .and_then(|value| value.parse().ok())
                .unwrap_or(default)
        };
        let yes = |key: &str| values.get(key).is_some_and(|value| value == "yes");
        Self::Edit(Box::new(Edit {
            name: avd.name.clone(),
            ram_mb: number("hw.ramSize", 2048),
            storage: values
                .get("disk.dataPartition.size")
                .cloned()
                .unwrap_or_else(|| "6G".to_owned()),
            cores: number("hw.cpu.ncore", 4),
            gpu: values
                .get("hw.gpu.mode")
                .cloned()
                .unwrap_or_else(|| "auto".to_owned()),
            keyboard: yes("hw.keyboard"),
            cold_boot: yes("fastboot.forceColdBoot"),
            avd: avd.clone(),
            error: None,
        }))
    }

    pub fn duplicate(avd: &Avd) -> Self {
        let name = format!("{} copy", avd.name);
        Self::Duplicate {
            id: avd::id_from_name(&name),
            name,
            avd: avd.clone(),
            job: None,
            error: None,
        }
    }

    pub fn delete(avd: &Avd) -> Self {
        Self::Delete {
            avd: avd.clone(),
            job: None,
            error: None,
        }
    }

    pub fn wipe(avd: &Avd) -> Self {
        Self::Wipe(avd.clone())
    }

    pub fn start(avd: &Avd, options: &launch::Options) -> Self {
        Self::Start {
            avd: avd.clone(),
            extra: options.extra.join(" "),
            options: options.clone(),
        }
    }

    pub fn snapshots(app: &App, avd: &Avd) -> Self {
        Self::Snapshots(Box::new(Snapshots {
            emulator: app.tab(&avd.id).and_then(|tab| tab.emulator()),
            avd: avd.clone(),
            list: None,
            snapshots: Vec::new(),
            name: String::new(),
            action: None,
            message: None,
        }))
    }
}

/// Shows `dialog`; returns it while it stays open.
pub fn show(app: &mut App, ctx: &egui::Context, dialog: Dialog) -> Option<Dialog> {
    match dialog {
        Dialog::Create(create) => create_dialog(app, ctx, create).map(Dialog::Create),
        Dialog::Edit(edit) => edit_dialog(app, ctx, edit).map(Dialog::Edit),
        Dialog::Duplicate {
            avd,
            mut name,
            mut id,
            mut job,
            mut error,
        } => {
            let mut open = true;
            if let Some(result) = job.as_ref().and_then(Job::take) {
                job = None;
                match result {
                    Ok(_) => {
                        app.rescan(true);
                        return None;
                    }
                    Err(message) => error = Some(message),
                }
            }
            modal(ctx, "duplicate", |ui| {
                ui.label(theme::title(format!("Duplicate {}", avd.name), 18.0));
                ui.label(
                    "The copy keeps the configuration and user data and cold boots the first time.",
                );
                egui::Grid::new("duplicate-form")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Name");
                        if ui.text_edit_singleline(&mut name).changed() {
                            id = avd::id_from_name(&name);
                        }
                        ui.end_row();
                        ui.label("Id");
                        ui.text_edit_singleline(&mut id);
                        ui.end_row();
                    });
                error_line(ui, &error);
                ui.horizontal(|ui| {
                    if job.is_some() {
                        ui.spinner();
                        ui.label("Copying…");
                    } else {
                        let valid = avd::valid_id(&id) && !name.trim().is_empty();
                        if ui
                            .add_enabled_ui(valid, |ui| theme::primary(ui, "Duplicate"))
                            .inner
                            .clicked()
                        {
                            let home = app.tools.avd_home.as_ref().map(|home| home.path.clone());
                            let (source, new_id, new_name) =
                                (avd.clone(), id.clone(), name.trim().to_owned());
                            job = Some(Job::spawn(ctx, move |_| {
                                let home = home.ok_or("no AVD folder")?;
                                avd::duplicate(&home, &source, &new_id, &new_name)
                                    .map_err(|error| error.to_string())
                            }));
                        }
                        if ui.button("Cancel").clicked() {
                            open = false;
                        }
                    }
                });
            });
            open.then_some(Dialog::Duplicate {
                avd,
                name,
                id,
                job,
                error,
            })
        }
        Dialog::Delete {
            avd,
            mut job,
            mut error,
        } => {
            let mut open = true;
            if let Some(result) = job.as_ref().and_then(Job::take) {
                job = None;
                match result {
                    Ok(()) => {
                        app.rescan(true);
                        return None;
                    }
                    Err(message) => error = Some(message),
                }
            }
            modal(ctx, "delete", |ui| {
                ui.label(theme::title(format!("Delete {}?", avd.name), 18.0));
                ui.label(format!(
                    "This removes {} and everything on the device. It cannot be undone.",
                    avd.dir.display()
                ));
                error_line(ui, &error);
                ui.horizontal(|ui| {
                    if job.is_some() {
                        ui.spinner();
                        ui.label("Deleting…");
                    } else {
                        if theme::danger(ui, format!("{}  Delete", icons::DELETE)).clicked() {
                            let home = app.tools.avd_home.as_ref().map(|home| home.path.clone());
                            let target = avd.clone();
                            job = Some(Job::spawn(ctx, move |_| {
                                let home = home.ok_or("no AVD folder")?;
                                avd::delete(&home, &target).map_err(|error| error.to_string())
                            }));
                        }
                        if ui.button("Cancel").clicked() {
                            open = false;
                        }
                    }
                });
            });
            open.then_some(Dialog::Delete { avd, job, error })
        }
        Dialog::Wipe(avd) => {
            let mut open = true;
            modal(ctx, "wipe", |ui| {
                ui.label(theme::title(format!("Wipe {}?", avd.name), 18.0));
                ui.label("The device starts as new: apps, accounts and files on it are erased.");
                ui.horizontal(|ui| {
                    if theme::danger(ui, "Wipe and start").clicked() {
                        let options = launch::Options {
                            wipe_data: true,
                            ..app.config.launch.clone()
                        };
                        app.start(ctx, &avd.id, &options);
                        open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });
            open.then_some(Dialog::Wipe(avd))
        }
        Dialog::Start {
            avd,
            mut options,
            mut extra,
        } => {
            let mut open = true;
            modal(ctx, "start", |ui| {
                ui.label(theme::title(format!("Start {}", avd.name), 18.0));
                ui.checkbox(&mut options.cold_boot, "Cold boot (ignore the saved state)");
                ui.checkbox(&mut options.wipe_data, "Wipe data (factory reset)");
                ui.checkbox(
                    &mut options.no_snapshot_save,
                    "Don't save the state on exit",
                );
                ui.horizontal(|ui| {
                    ui.label("Graphics");
                    egui::ComboBox::new("start-gpu", "")
                        .selected_text(options.gpu.as_deref().unwrap_or("AVD's setting"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut options.gpu, None, "AVD's setting");
                            for mode in GPU_MODES {
                                ui.selectable_value(&mut options.gpu, Some(mode.to_owned()), mode);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Extra flags");
                    ui.add(
                        egui::TextEdit::singleline(&mut extra)
                            .hint_text("-netdelay gprs -no-audio"),
                    );
                });
                ui.horizontal(|ui| {
                    if theme::primary(ui, format!("{}  Start", icons::START)).clicked() {
                        options.extra = extra.split_whitespace().map(str::to_owned).collect();
                        app.start(ctx, &avd.id, &options);
                        open = false;
                    }
                    if ui.button("Cancel").clicked() {
                        open = false;
                    }
                });
            });
            open.then_some(Dialog::Start {
                avd,
                options,
                extra,
            })
        }
        Dialog::Snapshots(snapshots) => {
            snapshots_dialog(app, ctx, snapshots).map(Dialog::Snapshots)
        }
    }
}

fn error_line(ui: &mut egui::Ui, error: &Option<String>) {
    if let Some(error) = error {
        ui.label(
            RichText::new(format!("{}  {error}", icons::WARNING))
                .color(Color32::from_rgb(220, 80, 80)),
        );
    }
}

fn create_dialog(app: &mut App, ctx: &egui::Context, mut form: Box<Create>) -> Option<Box<Create>> {
    let tools = app.tools.clone();
    let env = tools.child_env();
    let (Some(cmdline), Some(_jdk)) = (&tools.cmdline_tools, &tools.jdk) else {
        let mut open = true;
        modal(ctx, "create", |ui| {
            ui.label(theme::title("Create device", 18.0));
            ui.label("Creating devices needs the SDK command-line tools and a JDK. Set them in Settings.");
            if ui.button("OK").clicked() {
                open = false;
            }
        });
        return open.then_some(form);
    };
    let avdmanager = cmdline.path.join("avdmanager.bat");
    let sdkmanager = cmdline.path.join("sdkmanager.bat");

    // Device profiles come from avdmanager (a JVM: a few seconds), once.
    if form.devices.is_empty() && form.devices_job.is_none() {
        let (tool_path, env) = (avdmanager.clone(), env.clone());
        form.devices_job = Some(Job::spawn(ctx, move |_| {
            tool::run(&tool_path, &["list", "device", "-c"], &env, "")
                .map(|output| avd::device_ids(&output))
                .map_err(|error| error.to_string())
        }));
    }
    if let Some(result) = form.devices_job.as_ref().and_then(Job::take) {
        form.devices_job = None;
        match result {
            Ok(devices) => form.devices = devices,
            Err(message) => form.error = Some(message),
        }
    }
    if let Some(result) = form.available_job.as_ref().and_then(Job::take) {
        form.available_job = None;
        match result {
            Ok(available) => form.available = available,
            Err(message) => form.error = Some(message),
        }
    }
    if let Some(result) = form.install_job.as_ref().and_then(Job::take) {
        form.install_job = None;
        match result {
            Ok(()) => {
                if let Some(sdk) = &tools.sdk {
                    form.installed = images::installed(&sdk.path);
                }
                let installed = form.installed.clone();
                form.available
                    .retain(|image| !installed.iter().any(|done| done.package == image.package));
            }
            Err(message) => form.error = Some(message),
        }
    }
    if let Some(result) = form.job.as_ref().and_then(Job::take) {
        form.job = None;
        match result {
            Ok(_) => {
                app.rescan(true);
                return None;
            }
            Err(message) => form.error = Some(message),
        }
    }

    let mut open = true;
    modal(ctx, "create", |ui| {
        ui.set_min_width(520.0);
        ui.label(theme::title("Create device", 18.0));
        egui::Grid::new("create-form")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                if ui
                    .add(egui::TextEdit::singleline(&mut form.name).hint_text("Pixel 9 API 36"))
                    .changed()
                    && !form.id_edited
                {
                    form.id = avd::id_from_name(&form.name);
                }
                ui.end_row();
                ui.label("Id");
                if ui.text_edit_singleline(&mut form.id).changed() {
                    form.id_edited = true;
                }
                ui.end_row();

                ui.label("Device");
                ui.horizontal(|ui| {
                    egui::ComboBox::new("create-device", "")
                        .selected_text(&form.device)
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for device in form.devices.clone() {
                                ui.selectable_value(&mut form.device, device.clone(), device);
                            }
                        });
                    if form.devices_job.is_some() {
                        ui.spinner();
                    }
                });
                ui.end_row();

                ui.label("System image");
                ui.vertical(|ui| {
                    let selected = form
                        .image
                        .clone()
                        .unwrap_or_else(|| "none installed".to_owned());
                    egui::ComboBox::new("create-image", "")
                        .selected_text(short(&selected))
                        .width(360.0)
                        .show_ui(ui, |ui| {
                            for image in form.installed.clone() {
                                let label = format!(
                                    "API {}  ·  {}  ·  {}",
                                    image.api(),
                                    image.tag(),
                                    image.abi()
                                );
                                ui.selectable_value(
                                    &mut form.image,
                                    Some(image.package.clone()),
                                    label,
                                )
                                .on_hover_text(&image.description);
                            }
                        });
                    match (&form.install_job, &form.available_job) {
                        (Some(job), _) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                let text = images::progress(&job.progress())
                                    .map_or("Installing…".to_owned(), |(percent, what)| {
                                        format!("{percent}%  {what}")
                                    });
                                ui.label(RichText::new(text).small());
                            });
                        }
                        (None, Some(_)) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Asking sdkmanager for images…").small());
                            });
                        }
                        (None, None) if form.available.is_empty() => {
                            if ui
                                .small_button(format!("{}  Download more images…", icons::DOWNLOAD))
                                .clicked()
                            {
                                let (tool_path, env, installed) =
                                    (sdkmanager.clone(), env.clone(), form.installed.clone());
                                form.available_job = Some(Job::spawn(ctx, move |_| {
                                    tool::run(&tool_path, &["--list"], &env, "")
                                        .map(|output| images::available(&output, &installed))
                                        .map_err(|error| error.to_string())
                                }));
                            }
                        }
                        (None, None) => {
                            egui::ScrollArea::vertical()
                                .max_height(140.0)
                                .show(ui, |ui| {
                                    for image in form.available.clone() {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .small_button(icons::DOWNLOAD)
                                                .on_hover_text("Install")
                                                .clicked()
                                            {
                                                let (tool_path, env, package) = (
                                                    sdkmanager.clone(),
                                                    env.clone(),
                                                    image.package.clone(),
                                                );
                                                form.image = Some(image.package.clone());
                                                form.install_job =
                                                    Some(Job::spawn(ctx, move |progress| {
                                                        // Accept the licenses the image needs.
                                                        tool::run_with_progress(
                                                            &tool_path,
                                                            &[&package],
                                                            &env,
                                                            &"y\n".repeat(20),
                                                            |text| progress(text.to_owned()),
                                                        )
                                                        .map(drop)
                                                        .map_err(|error| error.to_string())
                                                    }));
                                            }
                                            ui.label(
                                                RichText::new(format!(
                                                    "API {}  ·  {}  ·  {}",
                                                    image.api(),
                                                    image.tag(),
                                                    image.abi()
                                                ))
                                                .small(),
                                            )
                                            .on_hover_text(&image.description);
                                        });
                                    }
                                });
                        }
                    }
                });
                ui.end_row();

                ui.label("Memory");
                ui.add(
                    egui::Slider::new(&mut form.ram_mb, 1024..=16384)
                        .step_by(512.0)
                        .suffix(" MB"),
                );
                ui.end_row();
                ui.label("Storage");
                ui.add(egui::Slider::new(&mut form.storage_gb, 2..=128).suffix(" GB"));
                ui.end_row();
            });
        error_line(ui, &form.error);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if form.job.is_some() {
                ui.spinner();
                ui.label("Creating…");
                return;
            }
            let exists = app.avds.iter().any(|avd| avd.id == form.id);
            let ready = avd::valid_id(&form.id)
                && !exists
                && form.image.is_some()
                && form.install_job.is_none()
                && !form.device.is_empty();
            let create = ui
                .add_enabled_ui(ready, |ui| {
                    theme::primary(ui, format!("{}  Create", icons::ADD))
                })
                .inner;
            if exists {
                ui.label(RichText::new("That id is taken.").small().weak());
            }
            if create.clicked() {
                let (tool_path, env) = (avdmanager.clone(), env.clone());
                let home = tools.avd_home.as_ref().map(|home| home.path.clone());
                let (id, name, device, package) = (
                    form.id.clone(),
                    if form.name.trim().is_empty() {
                        form.id.clone()
                    } else {
                        form.name.trim().to_owned()
                    },
                    form.device.clone(),
                    form.image.clone().unwrap_or_default(),
                );
                let (ram, storage) = (form.ram_mb, form.storage_gb);
                form.error = None;
                form.job = Some(Job::spawn(ctx, move |_| {
                    let mut args = vec!["create", "avd", "-n", &id, "-k", &package, "-d", &device];
                    let path;
                    if let Some(home) = &home {
                        path = home.join(format!("{id}.avd")).display().to_string();
                        args.extend(["-p", &path]);
                    }
                    // "Do you wish to create a custom hardware profile? [no]"
                    let output = tool::run(&tool_path, &args, &env, "no\n")
                        .map_err(|error| error.to_string())?;
                    let home = home.ok_or("no AVD folder")?;
                    let created = Avd::read(&home, &id)
                        .ok_or_else(|| format!("avdmanager did not create it:\n{output}"))?;
                    avd::write_values(
                        &created,
                        &[
                            ("avd.ini.displayname", name),
                            ("hw.ramSize", ram.to_string()),
                            ("disk.dataPartition.size", format!("{storage}G")),
                            ("hw.keyboard", "yes".to_owned()),
                        ],
                    )
                    .map_err(|error| error.to_string())?;
                    Ok(id)
                }));
            }
            if ui.button("Cancel").clicked() {
                open = false;
            }
        });
    });
    open.then_some(form)
}

/// `system-images;android-36;google_apis;x86_64` -> `API 36 · google_apis · x86_64`.
fn short(package: &str) -> String {
    let image = SystemImage {
        package: package.to_owned(),
        description: String::new(),
        installed: true,
    };
    if image.abi().is_empty() {
        package.to_owned()
    } else {
        format!(
            "API {}  ·  {}  ·  {}",
            image.api(),
            image.tag(),
            image.abi()
        )
    }
}

fn edit_dialog(app: &mut App, ctx: &egui::Context, mut form: Box<Edit>) -> Option<Box<Edit>> {
    let mut open = true;
    modal(ctx, "edit", |ui| {
        ui.label(theme::title(format!("Edit {}", form.avd.name), 18.0));
        egui::Grid::new("edit-form")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut form.name);
                ui.end_row();
                ui.label("Memory");
                ui.add(
                    egui::Slider::new(&mut form.ram_mb, 1024..=16384)
                        .step_by(512.0)
                        .suffix(" MB"),
                );
                ui.end_row();
                ui.label("Storage");
                ui.add(egui::TextEdit::singleline(&mut form.storage).hint_text("16G"));
                ui.end_row();
                ui.label("CPU cores");
                ui.add(egui::Slider::new(&mut form.cores, 1..=16));
                ui.end_row();
                ui.label("Graphics");
                egui::ComboBox::new("edit-gpu", "")
                    .selected_text(&form.gpu)
                    .show_ui(ui, |ui| {
                        for mode in GPU_MODES {
                            ui.selectable_value(&mut form.gpu, mode.to_owned(), mode);
                        }
                    });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut form.keyboard, "Hardware keyboard");
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut form.cold_boot, "Always cold boot");
                ui.end_row();
            });
        ui.label(
            RichText::new("Storage can only grow; a smaller size takes effect after Wipe data.")
                .small()
                .weak(),
        );
        error_line(ui, &form.error);
        ui.horizontal(|ui| {
            if theme::primary(ui, format!("{}  Save", icons::CHECK)).clicked() {
                let yes = |value: bool| if value { "yes" } else { "no" }.to_owned();
                let values = [
                    ("avd.ini.displayname", form.name.trim().to_owned()),
                    ("hw.ramSize", form.ram_mb.to_string()),
                    ("disk.dataPartition.size", form.storage.trim().to_owned()),
                    ("hw.cpu.ncore", form.cores.to_string()),
                    ("hw.gpu.mode", form.gpu.clone()),
                    ("hw.gpu.enabled", "yes".to_owned()),
                    ("hw.keyboard", yes(form.keyboard)),
                    ("fastboot.forceColdBoot", yes(form.cold_boot)),
                ];
                match avd::write_values(&form.avd, &values) {
                    Ok(()) => {
                        app.rescan(true);
                        open = false;
                    }
                    Err(error) => form.error = Some(error.to_string()),
                }
            }
            if ui.button("Cancel").clicked() {
                open = false;
            }
        });
    });
    open.then_some(form)
}

fn snapshots_dialog(
    app: &mut App,
    ctx: &egui::Context,
    mut form: Box<Snapshots>,
) -> Option<Box<Snapshots>> {
    let runtime = app.runtime();
    let Some(emulator) = form.emulator.clone() else {
        let mut open = true;
        modal(ctx, "snapshots", |ui| {
            ui.label(theme::title("Snapshots", 18.0));
            ui.label("Open the device in Avdeck first.");
            if ui.button("OK").clicked() {
                open = false;
            }
        });
        return open.then_some(form);
    };
    let refresh = |ctx: &egui::Context| {
        let (mut client, runtime) = (emulator.clone(), runtime.clone());
        Job::spawn(ctx, move |_| {
            runtime.block_on(async move {
                let list = client
                    .snapshots
                    .list_snapshots(SnapshotFilter {
                        status_filter: LoadStatus::All as i32,
                    })
                    .await
                    .map_err(|status| status.message().to_owned())?
                    .into_inner();
                Ok(list
                    .snapshots
                    .into_iter()
                    .map(|snapshot| {
                        let details = snapshot.details.unwrap_or_default();
                        let when = details
                            .creation_time
                            .map(|seconds| format!("{} ", age(seconds)))
                            .unwrap_or_default();
                        let size = format!("{:.1} GB", snapshot.size as f64 / 1e9);
                        (snapshot.snapshot_id, format!("{when}· {size}"))
                    })
                    .collect())
            })
        })
    };
    if form.list.is_none() && form.snapshots.is_empty() && form.message.is_none() {
        form.list = Some(refresh(ctx));
    }
    if let Some(result) = form.list.as_ref().and_then(Job::take) {
        form.list = None;
        match result {
            Ok(list) => form.snapshots = list,
            Err(message) => form.message = Some(message),
        }
    }
    if let Some(result) = form.action.as_ref().and_then(Job::take) {
        form.action = None;
        form.message = Some(result.unwrap_or_else(|error| error));
        form.list = Some(refresh(ctx));
    }

    let mut open = true;
    let mut action: Option<(&'static str, String)> = None;
    modal(ctx, "snapshots", |ui| {
        ui.set_min_width(480.0);
        ui.label(theme::title(
            format!("Snapshots of {}", form.avd.name),
            18.0,
        ));
        if form.list.is_some() {
            ui.spinner();
        }
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("snapshot-list")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for (id, details) in &form.snapshots {
                            ui.label(RichText::new(id).strong());
                            ui.label(RichText::new(details).small().weak());
                            ui.horizontal(|ui| {
                                if ui.button("Load").clicked() {
                                    action = Some(("load", id.clone()));
                                }
                                if ui.button(icons::DELETE).on_hover_text("Delete").clicked() {
                                    action = Some(("delete", id.clone()));
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("snapshot name"));
            let valid = avd::valid_id(&form.name);
            if ui
                .add_enabled(
                    valid,
                    egui::Button::new(format!("{}  Save now", icons::SNAPSHOT)),
                )
                .clicked()
            {
                action = Some(("save", form.name.clone()));
            }
        });
        if form.action.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working…");
            });
        }
        if let Some(message) = &form.message {
            ui.label(RichText::new(message).small());
        }
        if ui.button("Close").clicked() {
            open = false;
        }
    });
    if let Some((verb, id)) = action
        && form.action.is_none()
    {
        let (mut client, runtime) = (emulator.clone(), runtime.clone());
        form.action = Some(Job::spawn(ctx, move |_| {
            runtime.block_on(async move {
                let package = SnapshotPackage {
                    snapshot_id: id.clone(),
                    ..Default::default()
                };
                let reply = match verb {
                    "save" => client.snapshots.save_snapshot(package).await,
                    "load" => client.snapshots.load_snapshot(package).await,
                    _ => client.snapshots.delete_snapshot(package).await,
                }
                .map_err(|status| status.message().to_owned())?
                .into_inner();
                if reply.success {
                    Ok(format!("{verb}: {id} done"))
                } else {
                    Err(String::from_utf8_lossy(&reply.err).into_owned())
                }
            })
        }));
        form.name.clear();
    }
    open.then_some(form)
}

/// "5 min ago", "3 h ago", "2 days ago" for a Unix time in seconds.
fn age(seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |now| now.as_secs() as i64);
    let minutes = (now - seconds).max(0) / 60;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=2879 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}
