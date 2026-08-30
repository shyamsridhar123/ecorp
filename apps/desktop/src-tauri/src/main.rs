use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            #[cfg(any(target_os = "linux", windows))]
            app.deep_link().register_all()?;

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let urls = event.urls();
                let Some(url) = urls.first() else {
                    return;
                };
                if url.scheme() != "crony" {
                    return;
                }
                if let Some(window) = handle.get_webview_window("main")
                    && let Ok(payload) = serde_json::to_string(url.as_str())
                {
                    let _ = window.eval(format!(
                        "window.dispatchEvent(new CustomEvent('crony-deep-link', {{ detail: {payload} }}));"
                    ));
                    let _ = window.set_focus();
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run Crony desktop shell");
}
