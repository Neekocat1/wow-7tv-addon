//! Entry point.
//!
//! ```text
//! SevenTVEmotesHelper.exe            first run: install the addons, then run in the tray
//! ... --background                   started by Windows at login (no first-run messages)
//! ... --no-tray                      run in the console instead of the tray
//! ... --once                         one pass, then exit (handy for testing)
//! ... --wow-dir "D:/Games/World of Warcraft"   if WoW isn't found automatically
//! ... --flavor _retail_              also manage another game folder
//! ... --autostart on|off
//! ```
#![cfg_attr(windows, windows_subsystem = "windows")]

use seventv_helper::instance::{self, SingleInstance, vtuple};
use seventv_helper::paths::{AppPaths, Config, merge_unique};
use seventv_helper::service::Service;
use seventv_helper::seventv::UreqHttp;
use seventv_helper::wow::SystemProcesses;
use seventv_helper::{VERSION, installer, logger};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct Args {
    background: bool,
    no_tray: bool,
    once: bool,
    wow_dir: Vec<String>,
    flavor: Vec<String>,
    autostart: Option<bool>,
}

const USAGE: &str = "usage: SevenTVEmotesHelper [--background] [--no-tray] [--once] [--wow-dir DIR]... [--flavor NAME]... [--autostart on|off] [--version]";

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or(format!("{name} needs a value"))
        };
        match flag.as_str() {
            "--background" => a.background = true,
            "--no-tray" => a.no_tray = true,
            "--once" => a.once = true,
            "--wow-dir" => a.wow_dir.push(value("--wow-dir")?),
            "--flavor" => a.flavor.push(value("--flavor")?),
            "--autostart" => match value("--autostart")?.as_str() {
                "on" => a.autostart = Some(true),
                "off" => a.autostart = Some(false),
                other => return Err(format!("--autostart: expected on or off, not {other}")),
            },
            "--version" | "-V" => return Err(VERSION.to_string()),
            "--help" | "-h" => return Err(format!("7TV Emotes Helper {VERSION}\n{USAGE}")),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(a)
}

fn attach_console() {
    #[cfg(windows)]
    seventv_helper::win::attach_parent_console();
}

fn message_box(title: &str, text: &str) {
    #[cfg(windows)]
    seventv_helper::win::message_box(title, text);
    #[cfg(not(windows))]
    println!("{title}: {text}");
}

fn ask_folder(title: &str, text: &str) -> Option<String> {
    message_box(title, text);
    #[cfg(windows)]
    return seventv_helper::win::pick_folder("Select your World of Warcraft folder");
    #[cfg(not(windows))]
    None
}

fn first_run(service: &mut Service, interactive: bool) {
    service.discover();
    if service.libs.is_empty() && interactive {
        let picked = ask_folder(
            "7TV Emotes Helper",
            "I couldn't find World of Warcraft: Forever automatically.\n\n\
             Pick your World of Warcraft folder (the one that contains _forever_).",
        );
        if let Some(p) = picked {
            merge_unique(&mut service.config.wow_dirs, &[p]);
            service.discover();
        }
    }
    // Starting with Windows is set up by the installer (and the tray toggle), not here.
    service.config.first_run_done = true;
    let _ = service.config.save(&service.paths);
    if interactive {
        if !service.libs.is_empty() {
            let where_: Vec<String> = service
                .libs
                .keys()
                .map(|f| format!("  {}", f.display()))
                .collect();
            message_box(
                "7TV Emotes Helper",
                &format!(
                    "All set! The 7TV Emotes addon is installed in:\n\n{}\n\n\
                     The helper now runs quietly in your system tray. In game, type /7tv to get started.\n\n\
                     If WoW is open, restart it once.",
                    where_.join("\n")
                ),
            );
        } else {
            message_box(
                "7TV Emotes Helper",
                "World of Warcraft wasn't found, so nothing was installed yet. The helper will keep \
                 looking in the background; you can also run it with --wow-dir.",
            );
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            attach_console();
            println!("{msg}");
            std::process::exit(if msg == VERSION || msg.starts_with("7TV") {
                0
            } else {
                2
            });
        }
    };
    let console = args.no_tray || args.once || args.autostart.is_some();
    if console {
        attach_console();
    }
    let paths = AppPaths::new(AppPaths::default_dir());
    logger::init(paths.log_file(), args.no_tray || args.once);

    let mut cfg = Config::load(&paths);
    if !args.wow_dir.is_empty() || !args.flavor.is_empty() {
        merge_unique(&mut cfg.wow_dirs, &args.wow_dir);
        merge_unique(&mut cfg.flavors, &args.flavor);
        let _ = cfg.save(&paths);
    }

    if let Some(on) = args.autostart {
        match installer::set_autostart(on) {
            Ok(()) => println!("autostart {}", if on { "on" } else { "off" }),
            Err(e) => println!("couldn't change autostart: {e}"),
        }
        return;
    }

    let mut service = Service::new(
        cfg,
        paths.clone(),
        Arc::new(UreqHttp::new()),
        Box::new(SystemProcesses::new()),
    );

    // Only one helper at a time. A newer helper asks an older running one to quit and takes over.
    let mut lock = SingleInstance::new(&paths);
    let (got, other) = instance::take_over(&paths, &mut lock, VERSION, Duration::from_secs(15));
    if !got {
        let msg = match &other {
            Some(o) if vtuple(o) > vtuple(VERSION) => {
                format!("A newer helper ({o}) is already running; this one ({VERSION}) won't replace it.")
            }
            _ => "The helper is already running and didn't close when asked (versions before 0.5 can't be \
                  asked). Right-click the 7TV icon in your system tray, choose Quit, then start this again."
                .to_string(),
        };
        if args.once || args.no_tray {
            println!("{msg}");
        } else if !args.background {
            message_box("7TV Emotes Helper", &msg);
        }
        return;
    }
    if let Some(o) = &other {
        log::info!("took over from helper {o}");
    }
    #[cfg(windows)]
    let _running_marker = seventv_helper::win::running_mutex();

    if args.once {
        for r in service.run_once() {
            println!("{}{}", if r.ok { "OK   " } else { "FAIL " }, r.msg);
        }
        if service.libs.is_empty() {
            println!(
                "World of Warcraft: Forever wasn't found. Use --wow-dir to point at your WoW folder."
            );
        }
        for (flavor, lib) in &service.libs {
            let st = &lib.state;
            println!(
                "{}: {} emotes, {} textures ({}), {} waiting for a WoW restart",
                flavor.display(),
                lib.desired().len(),
                st.images.len(),
                st.format,
                st.waiting.len()
            );
            if let Some(n) = lib.notice() {
                println!("  {n}");
            }
        }
        println!(
            "If WoW was open while this ran for the first time, restart it once so it sees the new files."
        );
        lock.release();
        return;
    }

    instance::write_running(&paths, VERSION);
    log::info!("7TV Emotes Helper {VERSION} starting");
    if !service.config.first_run_done {
        first_run(&mut service, !args.background && !args.no_tray);
    }

    #[cfg(windows)]
    if !args.no_tray {
        seventv_helper::tray::run_tray(service);
        return;
    }

    // Console mode.
    *service.shared.notify.lock().unwrap() = Box::new(|title, msg| println!("[{title}] {msg}"));
    service.run();
}
