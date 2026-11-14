mod config;
mod kdl;
mod ui;

use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;
use ui::Ctx;
use ui::pointer::Device;

const APP_ID: &str = "io.github.NiriSettings";

const CSS: &str = r#"
.keycap {
    min-width: 18px;
    padding: 2px 7px;
    border-radius: 6px;
    font-size: 0.85em;
    font-weight: 600;
    background: alpha(currentColor, 0.08);
    box-shadow: inset 0 -2px alpha(currentColor, 0.15);
}
"#;

struct PageDef {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    build: fn(&Ctx) -> gtk::Widget,
}

const PAGES: &[PageDef] = &[
    PageDef { id: "keyboard", title: "Keyboard", icon: "input-keyboard-symbolic", build: |c| ui::keyboard::page(c).upcast() },
    PageDef { id: "mouse", title: "Mouse", icon: "input-mouse-symbolic", build: |c| ui::pointer::page(c, Device::Mouse).upcast() },
    PageDef { id: "touchpad", title: "Touchpad", icon: "input-touchpad-symbolic", build: |c| ui::pointer::page(c, Device::Touchpad).upcast() },
    PageDef { id: "trackpoint", title: "Trackpoint", icon: "input-mouse-symbolic", build: |c| ui::pointer::page(c, Device::Trackpoint).upcast() },
    PageDef { id: "trackball", title: "Trackball", icon: "input-mouse-symbolic", build: |c| ui::pointer::page(c, Device::Trackball).upcast() },
    PageDef { id: "general", title: "Focus & Cursor", icon: "find-location-symbolic", build: |c| ui::general::page(c).upcast() },
    PageDef { id: "shortcuts", title: "Shortcuts", icon: "preferences-desktop-keyboard-shortcuts-symbolic", build: ui::keybinds::page },
    PageDef { id: "fingerprint", title: "Fingerprint", icon: "auth-fingerprint-symbolic", build: |c| ui::fingerprint::page(c).upcast() },
];

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(build_window);
    app.run()
}

fn build_window(app: &adw::Application) {
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Niri Settings")
        .default_width(1000)
        .default_height(720)
        .width_request(360)
        .height_request(400)
        .build();
    let toasts = adw::ToastOverlay::new();

    let store = match config::ConfigStore::load() {
        Ok(s) => s,
        Err(e) => {
            let status = adw::StatusPage::builder()
                .icon_name("dialog-error-symbolic")
                .title("Couldn't read the niri config")
                .description(glib::markup_escape_text(&e).as_str())
                .build();
            let tv = adw::ToolbarView::new();
            tv.add_top_bar(&adw::HeaderBar::new());
            tv.set_content(Some(&status));
            window.set_content(Some(&tv));
            window.present();
            return;
        }
    };
    let ctx = Ctx { store: Rc::new(RefCell::new(store)), window: window.clone(), toasts: toasts.clone() };

    // Sidebar.
    let sidebar_list = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
    for p in PAGES {
        let b = gtk::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        b.append(&gtk::Image::from_icon_name(p.icon));
        b.append(&gtk::Label::builder().label(p.title).xalign(0.0).build());
        sidebar_list.append(&gtk::ListBoxRow::builder().child(&b).build());
    }
    let sidebar_tv = adw::ToolbarView::new();
    let sidebar_header = adw::HeaderBar::new();
    let menu = gio::Menu::new();
    menu.append(Some("Reload From Disk"), Some("win.reload"));
    menu.append(Some("Open Config Folder"), Some("win.open-folder"));
    menu.append(Some("Open Backups Folder"), Some("win.open-backups"));
    sidebar_header.pack_end(
        &gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).primary(true).build(),
    );
    sidebar_tv.add_top_bar(&sidebar_header);
    sidebar_tv.set_content(Some(
        &gtk::ScrolledWindow::builder().child(&sidebar_list).hscrollbar_policy(gtk::PolicyType::Never).build(),
    ));
    let sidebar_page = adw::NavigationPage::builder().title("Niri Settings").child(&sidebar_tv).build();

    // Content.
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    let content_tv = adw::ToolbarView::new();
    let content_header = adw::HeaderBar::new();
    let reload_btn = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Reload config from disk")
        .action_name("win.reload")
        .build();
    content_header.pack_end(&reload_btn);
    content_tv.add_top_bar(&content_header);
    content_tv.set_content(Some(&stack));
    let content_page = adw::NavigationPage::builder().title(PAGES[0].title).child(&content_tv).build();

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content_page)
        .min_sidebar_width(200.0)
        .build();
    toasts.set_child(Some(&split));
    window.set_content(Some(&toasts));

    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        640.0,
        adw::LengthUnit::Sp,
    ));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(bp);

    let build_pages = {
        let ctx = ctx.clone();
        let stack = stack.clone();
        move || {
            let visible = stack.visible_child_name();
            while let Some(c) = stack.first_child() {
                stack.remove(&c);
            }
            for p in PAGES {
                stack.add_named(&(p.build)(&ctx), Some(p.id));
            }
            if let Some(v) = visible {
                stack.set_visible_child_name(&v);
            }
        }
    };
    build_pages();

    {
        let stack = stack.clone();
        let split = split.clone();
        let content_page = content_page.clone();
        sidebar_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let p = &PAGES[row.index() as usize];
            stack.set_visible_child_name(p.id);
            content_page.set_title(p.title);
            split.set_show_content(true);
        });
    }
    sidebar_list.select_row(sidebar_list.row_at_index(0).as_ref());

    // Actions.
    let reload = gio::SimpleAction::new("reload", None);
    {
        let ctx = ctx.clone();
        reload.connect_activate(move |_, _| {
            let res = ctx.store.borrow_mut().reload();
            match res {
                Ok(()) => {
                    build_pages();
                    ctx.toast("Reloaded config");
                }
                Err(e) => ctx.error("Couldn't reload config", &e),
            }
        });
    }
    window.add_action(&reload);

    let open_folder = gio::SimpleAction::new("open-folder", None);
    {
        let ctx = ctx.clone();
        open_folder.connect_activate(move |_, _| {
            let dir = ctx.store.borrow().main.parent().map(|p| p.to_path_buf());
            if let Some(dir) = dir {
                open_uri(&ctx, &dir);
            }
        });
    }
    window.add_action(&open_folder);

    let open_backups = gio::SimpleAction::new("open-backups", None);
    {
        let ctx = ctx.clone();
        open_backups.connect_activate(move |_, _| {
            let dir = ctx.store.borrow().backup_dir().parent().map(|p| p.to_path_buf());
            if let Some(dir) = dir {
                if !dir.exists() {
                    ctx.toast("No backups yet — they're made the first time a file is changed");
                    return;
                }
                open_uri(&ctx, &dir);
            }
        });
    }
    window.add_action(&open_backups);

    window.present();

    // Developer aid: render every page to PNGs in this directory, then quit.
    if let Some(dir) = std::env::var_os("NIRI_SETTINGS_SCREENSHOT") {
        let dir = std::path::PathBuf::from(dir);
        let step = Rc::new(std::cell::Cell::new(0usize));
        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
            let i = step.get();
            if i > 0 {
                save_png(&window, &dir.join(format!("{}.png", PAGES[i - 1].id)));
            }
            if i == PAGES.len() {
                window.close();
                return glib::ControlFlow::Break;
            }
            sidebar_list.select_row(sidebar_list.row_at_index(i as i32).as_ref());
            step.set(i + 1);
            glib::ControlFlow::Continue
        });
    }
}

fn save_png(window: &adw::ApplicationWindow, path: &std::path::Path) {
    let (w, h) = (window.width() as f64, window.height() as f64);
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w, h);
    let Some(node) = snapshot.to_node() else { return };
    let Some(renderer) = window.renderer() else { return };
    let texture = renderer.render_texture(node, None);
    let _ = texture.save_to_png(path);
}

fn open_uri(ctx: &Ctx, dir: &std::path::Path) {
    let file = gio::File::for_path(dir);
    let launcher = gtk::FileLauncher::new(Some(&file));
    let ctx = ctx.clone();
    launcher.launch(Some(&ctx.window.clone()), gio::Cancellable::NONE, move |res| {
        if let Err(e) = res {
            ctx.error("Couldn't open folder", &e.to_string());
        }
    });
}
