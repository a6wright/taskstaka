//! The menu bar item and the global hotkey. OS-facing and HTTP-only: it
//! follows the daemon over `/api/v1/plate`, `/api/v1/plan` and the changes
//! long-poll, and every click is a command or a URL (ADR 0015).

mod actions;
mod daemon;
mod hotkey;
mod icon;
mod menu;
mod panel;

use std::path::PathBuf;

use clap::Parser;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use taskstaka_client::Client;
use taskstaka_core::{Command, Context};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};
use wry::{WebView, WebViewBuilder};

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
    /// The quick page, inside the panel, talking to us.
    Panel(panel::Ipc),
}

/// The Spotlight-style quick-add window: created hidden at start, shown by
/// the hotkey or the menu, hidden by Esc, an add, or losing focus.
struct Panel {
    window: Window,
    webview: WebView,
    /// The daemon URL the page was loaded from, so a daemon that comes up
    /// later (or on another port) gets the page reloaded.
    loaded_from: Option<String>,
}

impl Panel {
    fn create(
        target: &EventLoopWindowTarget<UserEvent>,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Panel, String> {
        let g = panel::geometry(screen_of(target));
        let builder = WindowBuilder::new()
            .with_title("Quick add")
            .with_inner_size(LogicalSize::new(g.width, g.height))
            .with_position(LogicalPosition::new(g.x, g.y))
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_minimizable(false)
            .with_closable(false)
            .with_visible_on_all_workspaces(true)
            .with_visible(false);
        let window = builder.build(target).map_err(|e| e.to_string())?;
        let wv = WebViewBuilder::new()
            .with_url("about:blank")
            .with_transparent(true)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                if let Some(msg) = panel::ipc(req.body()) {
                    let _ = proxy.send_event(UserEvent::Panel(msg));
                }
            });
        #[cfg(not(target_os = "linux"))]
        let webview = wv.build(&window).map_err(|e| e.to_string())?;
        #[cfg(target_os = "linux")]
        let webview = {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            let vbox = window
                .default_vbox()
                .ok_or("no gtk box on the panel window")?;
            wv.build_gtk(vbox).map_err(|e| e.to_string())?
        };
        Ok(Panel {
            window,
            webview,
            loaded_from: None,
        })
    }

    fn visible(&self) -> bool {
        self.window.is_visible()
    }

    /// Show on the primary screen, over everything, with the field focused.
    fn show(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        base: &str,
        ctx: Option<Context>,
    ) {
        if self.loaded_from.as_deref() != Some(base) {
            let url = panel::url(base, ctx);
            if let Err(e) = self.webview.load_url(&url) {
                eprintln!("taskstaka-tray: panel: {e}");
            }
            self.loaded_from = Some(base.to_string());
        }
        let g = panel::geometry(screen_of(target));
        self.window
            .set_outer_position(LogicalPosition::new(g.x, g.y));
        // Order matters: set_focus activates the app only once the window
        // is visible (an Accessory app has no other way to take the keyboard).
        self.window.set_visible(true);
        self.window.set_focus();
        let _ = self.webview.focus();
        let _ = self
            .webview
            .evaluate_script("window.taskstakaReset && window.taskstakaReset()");
    }

    /// Idempotent: hiding resigns key, which asks to hide again.
    fn hide(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        }
    }
}

/// The primary monitor in logical points, where the panel goes.
fn screen_of(target: &EventLoopWindowTarget<UserEvent>) -> panel::Screen {
    match target.primary_monitor() {
        Some(m) => {
            let sf = m.scale_factor();
            let size = m.size().to_logical::<f64>(sf);
            let pos = m.position().to_logical::<f64>(sf);
            panel::Screen {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            }
        }
        None => panel::Screen {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        },
    }
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
    let mut panel: Option<Panel> = None;
    let mut base: Option<String> = None;
    let mut spec = menu::build(&Model::Offline, &hotkey_label);
    let ctx = args.context;
    let panel_proxy = event_loop.create_proxy();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            // macOS wants the status item created once the loop is live.
            Event::NewEvents(StartCause::Init) => {
                let b = TrayIconBuilder::new()
                    .with_menu(Box::new(to_menu(&spec)))
                    .with_title(&spec.title)
                    .with_tooltip("taskstaka");
                // The brand mark: a template image on macOS (drawn as a mask,
                // so it matches the other status items in light and dark).
                #[cfg(target_os = "macos")]
                let b = b.with_icon_templated(icon::tray_icon());
                #[cfg(not(target_os = "macos"))]
                let b = b.with_icon(icon::tray_icon());
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
                match Panel::create(target, panel_proxy.clone()) {
                    Ok(p) => {
                        eprintln!("taskstaka-tray: quick-add panel ready");
                        panel = Some(p);
                    }
                    Err(e) => eprintln!(
                        "taskstaka-tray: no quick-add panel ({e}); the hotkey opens the browser instead"
                    ),
                }
            }
            // Esc, or an add finished: hide the panel.
            Event::UserEvent(UserEvent::Panel(panel::Ipc::Close)) => {
                if let Some(p) = &panel {
                    p.hide();
                }
            }
            Event::UserEvent(UserEvent::Panel(panel::Ipc::Added)) => {}
            // Clicking elsewhere hides it, like Spotlight.
            Event::WindowEvent {
                event: WindowEvent::Focused(false) | WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(p) = &panel {
                    p.hide();
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
                // Toggle, like Spotlight.
                match &panel {
                    Some(p) if p.visible() => p.hide(),
                    _ => quick_add(&mut panel, target, &mut base, &dir, args.port, ctx),
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if let Some(p) = &panel {
                    p.hide();
                }
                let b = base.clone().unwrap_or_default();
                match actions::effect(e.id.as_ref(), &b) {
                    Some(Effect::Panel) => {
                        quick_add(&mut panel, target, &mut base, &dir, args.port, ctx)
                    }
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

/// Quick add: the panel when we have one, else the quick page in the browser.
fn quick_add(
    panel: &mut Option<Panel>,
    target: &EventLoopWindowTarget<UserEvent>,
    base: &mut Option<String>,
    dir: &std::path::Path,
    port: Option<u16>,
    ctx: Option<Context>,
) {
    if !ensure_daemon(base, dir, port) {
        return;
    }
    let Some(b) = base.as_deref() else { return };
    match panel {
        Some(p) => p.show(target, b, ctx),
        None => open_url(&format!("{b}/quick")),
    }
}

/// Open a daemon page in the browser, (re)connecting first so the link works
/// even right after a `taskstaka stop`.
fn open_page(base: &mut Option<String>, dir: &std::path::Path, port: Option<u16>, path: &str) {
    if ensure_daemon(base, dir, port)
        && let Some(b) = base
    {
        open_url(&format!("{b}{path}"));
    }
}

/// Make sure a daemon answers (starting one if the user stopped it) and
/// remember its URL. `false` when none could be reached.
fn ensure_daemon(base: &mut Option<String>, dir: &std::path::Path, port: Option<u16>) -> bool {
    if base.is_none() || Client::discover(dir).is_none() {
        match Client::connect(dir, port) {
            Ok(c) => *base = Some(c.base().to_string()),
            Err(e) => {
                eprintln!("taskstaka-tray: {e:#}");
                return false;
            }
        }
    }
    base.is_some()
}

fn open_url(url: &str) {
    if let Err(e) = open::that_detached(url) {
        eprintln!("taskstaka-tray: opening {url}: {e}");
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
