//! Mailix — a fast, lightweight, IMAP-first desktop mail client.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod ai;
mod app;
mod config;
mod contacts;
mod grammar;
mod imap;
mod markup;
mod mime;
mod net;
mod search;
mod secrets;
mod signature;
mod smtp;
mod store;
mod theme;
mod thread;
mod view;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mailix=info,warn".into()),
        )
        .init();
    rustls::crypto::ring::default_provider().install_default().ok();
    config::migrate_old_name();

    let window = iced::window::Settings {
        size: iced::Size::new(1280.0, 800.0),
        min_size: Some(iced::Size::new(760.0, 480.0)),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "mailix".into(),
            ..Default::default()
        },
        ..Default::default()
    };

    iced::application(app::App::boot, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .font(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-Italic.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/lucide.ttf").as_slice())
        .default_font(view::UI)
        .window(window)
        .run()
}
