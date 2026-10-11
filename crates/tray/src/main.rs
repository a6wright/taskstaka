//! The menu bar item and the global hotkey. OS-facing and HTTP-only: it
//! follows the daemon over `/api/v1/plate`, `/api/v1/plan` and the changes
//! long-poll, and every click is a command or a URL (ADR 0015).

mod actions;
mod daemon;
mod hotkey;
mod menu;

use std::path::PathBuf;

use clap::Parser;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use taskstaka_client::Client;
use taskstaka_core::{Command, Context};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use actions::Effect;
use menu::{Item, MenuSpec, Model};

/// taskstaka in the menu bar: the task in hand, what you can do now, and a
/// hotkey for quick add.
#[derive(Parser, Debug)]
#[command(name = "taskstaka-tray", version)]
struct Args {
    /// Notes folder (default: the one in Settings, else ~/todo).
    #[arg(long, env = "TASKSTAKA_DIR")]
    dir: Option<PathBuf>,
    /// Daemon port (default 7407).
    #[arg(long, env = "TASKSTAKA_PORT")]
    port: Option<u16>,
    /// Show one context only (work | personal).
    #[arg(long, env = "TASKSTAKA_CONTEXT")]
    context: Option<Context>,
    /// Global key for quick add, e.g. alt+space, ctrl+alt+space, cmd+shift+t.
    #[arg(long, env = "TASKSTAKA_HOTKEY", default_value = "alt+space")]
    hotkey: String,
}

enum UserEvent {
    Daemon(daemon::Msg),
    Menu(MenuEvent),
    HotKey(GlobalHotKeyEvent),
}

fn main() {
    let args = Args::parse();
    let dir = args.dir.clone().unwrap_or_else(default_dir);
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // No Dock icon, no app menu: a status item only.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    // Every library channel is forwarded into the one event loop, so
    // ControlFlow::Wait is enough and nothing polls.
    let p = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::Menu(e));
    }));
    let p = event_loop.create_proxy();
    GlobalHotKeyEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::HotKey(e));
    }));

    // The hotkey: main thread, before the loop runs. A combo another app
    // owns is reported, not fatal — the menu still works.
    let hotkey_label = hotkey::label(&args.hotkey);
    let manager = GlobalHotKeyManager::new().ok();
    let hotkey_id = match (&manager, hotkey::parse(&args.hotkey)) {
        (Some(m), Ok(hk)) => match m.register(hk) {
            Ok(()) => Some(hk.id()),
            Err(e) => {
                eprintln!("taskstaka-tray: could not register {}: {e}", args.hotkey);
                None
            }
        },
        (_, Err(e)) => {
            eprintln!("taskstaka-tray: {e}");
            None
        }
        (None, _) => {
            eprintln!("taskstaka-tray: global hotkeys unavailable here");
            None
        }
    };

    spawn_follower(&event_loop, dir.clone(), args.port, args.context);

    let mut tray: Option<TrayIcon> = None;
    let mut base: Option<String> = None;
    let mut spec = menu::build(&Model::Offline, &hotkey_label);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // macOS wants the status item created once the loop is live.
            Event::NewEvents(StartCause::Init) => {
                let b = TrayIconBuilder::new()
                    .with_menu(Box::new(to_menu(&spec)))
                    .with_title(&spec.title)
                    .with_tooltip("taskstaka");
                #[cfg(target_os = "linux")]
                let b = b.with_icon(placeholder_icon());
                match b.build() {
                    Ok(t) => tray = Some(t),
                    Err(e) => {
                        eprintln!("taskstaka-tray: no status item: {e}");
                        *control_flow = ControlFlow::ExitWithCode(1);
                        return;
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    use objc2_core_foundation::CFRunLoop;
                    if let Some(rl) = CFRunLoop::main() {
                        rl.wake_up();
                    }
                }
            }
            Event::UserEvent(UserEvent::Daemon(daemon::Msg::Connected { base: b })) => {
                eprintln!("taskstaka-tray: daemon at {b}");
                base = Some(b);
            }
            Event::UserEvent(UserEvent::Daemon(daemon::Msg::Model(model))) => {
                spec = menu::build(&model, &hotkey_label);
                eprintln!("taskstaka-tray: {}", spec.title);
                if let Some(t) = &tray {
                    t.set_title(Some(&spec.title));
                    t.set_menu(Some(Box::new(to_menu(&spec))));
                }
            }
            Event::UserEvent(UserEvent::HotKey(e))
                if Some(e.id) == hotkey_id && e.state == HotKeyState::Pressed =>
            {
                open_page(&mut base, &dir, args.port, "/quick");
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                let b = base.clone().unwrap_or_default();
                match actions::effect(e.id.as_ref(), &b) {
                    Some(Effect::Quit) => {
                        tray.take();
                        *control_flow = ControlFlow::Exit;
                    }
                    Some(Effect::Open(url)) => {
                        // Quick add and the web need a daemon: start one if
                        // the user stopped it.
                        let path = url.strip_prefix(&b).unwrap_or("/next").to_string();
                        open_page(&mut base, &dir, args.port, &path);
                    }
                    Some(Effect::SetStatus {
                        id,
                        status,
                        cascade,
                    }) => {
                        if let Some(b) = &base {
                            let client = Client::at(strip_scheme(b));
                            let Ok(id) = id.parse() else { return };
                            // Off the UI thread; the long-poll redraws the menu.
                            std::thread::spawn(move || {
                                if let Err(e) = client.apply(&Command::SetStatus {
                                    id,
                                    status,
                                    waiting_on: None,
                                    until: None,
                                    cascade,
                                }) {
                                    eprintln!("taskstaka-tray: {e:#}");
                                }
                            });
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    })
}

fn spawn_follower(
    event_loop: &tao::event_loop::EventLoop<UserEvent>,
    dir: PathBuf,
    port: Option<u16>,
    ctx: Option<Context>,
) {
    let proxy: EventLoopProxy<UserEvent> = event_loop.create_proxy();
    std::thread::spawn(move || {
        daemon::follow(&dir, port, ctx, |msg| {
            proxy.send_event(UserEvent::Daemon(msg)).is_ok()
        });
    });
}

/// Open a daemon page in the browser, (re)connecting first so the link works
/// even right after a `taskstaka stop`.
fn open_page(base: &mut Option<String>, dir: &std::path::Path, port: Option<u16>, path: &str) {
    if base.is_none() || Client::discover(dir).is_none() {
        match Client::connect(dir, port) {
            Ok(c) => *base = Some(c.base().to_string()),
            Err(e) => {
                eprintln!("taskstaka-tray: {e:#}");
                return;
            }
        }
    }
    if let Some(b) = base {
        let url = format!("{b}{path}");
        if let Err(e) = open::that_detached(&url) {
            eprintln!("taskstaka-tray: opening {url}: {e}");
        }
    }
}

/// `Client::at` wants `host:port`; `base()` gives `http://host:port`.
fn strip_scheme(base: &str) -> &str {
    base.strip_prefix("http://").unwrap_or(base)
}

/// The notes folder when `--dir`/`$TASKSTAKA_DIR` say nothing: the one in
/// the machine settings, else `~/todo` (same order as the CLI, ADR 0011).
fn default_dir() -> PathBuf {
    if let Some(d) = taskstaka_core::settings::Settings::load().dir {
        return d;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("todo")
}

/// A `MenuSpec` as muda items. Labels are disabled items: greyed, unclickable.
fn to_menu(spec: &MenuSpec) -> Menu {
    let menu = Menu::new();
    for item in &spec.items {
        let r = match item {
            Item::Label(t) => menu.append(&MenuItem::new(t, false, None)),
            Item::Action { id, text } => {
                menu.append(&MenuItem::with_id(id.as_str(), text, true, None))
            }
            Item::Separator => menu.append(&PredefinedMenuItem::separator()),
        };
        if let Err(e) = r {
            eprintln!("taskstaka-tray: menu: {e}");
        }
    }
    menu
}

/// AppIndicator hides title-only items, so Linux gets a plain mark until a
/// real icon lands.
#[cfg(target_os = "linux")]
fn placeholder_icon() -> tray_icon::Icon {
    let (w, h) = (16u32, 16u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        rgba.extend_from_slice(&[0x58, 0x7c, 0xff, 0xff]);
    }
    tray_icon::Icon::from_rgba(rgba, w, h).expect("placeholder icon")
}
