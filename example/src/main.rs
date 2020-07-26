use serde::{Deserialize, Serialize};
use webviewbuilder_win::{ReceiveWebviewMessage, ShowWebview, WebViewBuilder};
use winit::event::{Event, WindowEvent};
use winit::{
    dpi::LogicalSize,
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

#[derive(Serialize, Deserialize, Clone, Eq, PartialEq, Debug)]
#[serde(tag = "type")]
enum MsgFromWebView {
    HelloToServer,
    OpenOptionalWindow,
}

impl ReceiveWebviewMessage<AppEvent> for MsgFromWebView {
    fn pass_to_event_loop_proxy(self: Self, proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
        let _ = proxy.send_event(AppEvent::WindowMsg(self));
    }
}

#[derive(Serialize, Deserialize, Clone, Eq, PartialEq, Debug)]
#[serde(tag = "type")]
enum MsgToWebView {
    HelloToWebview,
}

#[derive(Clone, Eq, PartialEq, Debug)]
enum AppEvent {
    WindowMsg(MsgFromWebView),
}

fn main() {
    let event_loop = EventLoop::<AppEvent>::with_user_event();
    let proxy = event_loop.create_proxy();

    // Example of webview that does not need message passing
    let web1 = WebViewBuilder::new()
        .webview_init(|w| {
            w.navigate_to_string(
                r#"
                    <html>
                    <title>Foo</title>
                    <body>
                    <h2>WebView2 - No communication</h2>
                    "#,
            )
        })
        .build(&event_loop)
        .unwrap();

    // Example of webview that exist only optionally (like preference dialog)
    let mut webopt = WebViewBuilder::new()
        .webview_init(|w| {
            w.navigate_to_string(
                r#"
                    <html style="background: green;">
                    <title>Optional Window</title>
                    <body>
                    <h2>This exists only while it's open</h2>
                    "#,
            )
        })
        .build_optional(&event_loop);

    // Example of webview that has one-way communication
    let web2 = WebViewBuilder::new()
        .msg_from_webview::<MsgFromWebView>()
        .webview_init(|w| {
            w.navigate_to_string(
                r#"
                    <html>
                    <title>Foo</title>
                    <body>
                    <h2>WebView2 - One sided communication</h2>
                    <button type="button" onclick='window.chrome.webview.postMessage(JSON.stringify({"type": "OpenOptionalWindow"}));'>Open Optional Window</button>
                "#,
            )
        })
        .build(&event_loop)
        .unwrap();

    // Example of webview that has two-way communication
    let web3 = WebViewBuilder::new()
        .msg_from_webview::<MsgFromWebView>()
        .msg_to_webview::<MsgToWebView>()
        .webview_init(|w| {
            w.navigate_to_string(
                r#"
                    <html>
                    <title>Foo</title>
                    <body>
                    <h2>WebView2 - Host Web Communication</h2>
                    <p>Got messages:</p>
                    <script>
                        // Send to server
                        window.chrome.webview.postMessage('{ "type" : "HelloToServer" }'); 

                        // Receive messages from the server
                        chrome.webview.addEventListener("message", e => {
                            document.body.append(JSON.stringify(e.data));
                        });
                    </script>
                "#,
            )
        })
        // Optionally give window builder
        .window_builder(
            WindowBuilder::new()
                .with_resizable(false)
                .with_inner_size(LogicalSize::new(600, 600)),
        )
        // Give some settings
        .settings(|settings| {
            settings.put_is_status_bar_enabled(false)?;
            settings.put_are_default_context_menus_enabled(false)?;
            settings.put_is_zoom_control_enabled(false)?;
            settings.put_are_dev_tools_enabled(true)
        })
        .build(&event_loop)
        .unwrap();

    event_loop.run(move |event, event_loop_target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, window_id } => {
                let _ = web1.handle_window_event(&event, &window_id);
                let _ = web2.handle_window_event(&event, &window_id);
                let _ = web3.handle_window_event(&event, &window_id);
                let _ = webopt.handle_window_event(&event, &window_id);

                // Close the application if any of the webviews is closed
                if web1.is_window(&window_id)
                    || web2.is_window(&window_id)
                    || web3.is_window(&window_id)
                {
                    if let WindowEvent::CloseRequested = event {
                        *control_flow = ControlFlow::Exit
                    }
                }
            }
            Event::UserEvent(e) => match e {
                AppEvent::WindowMsg(m) => match m {
                    MsgFromWebView::HelloToServer => {
                        println!("Got Hello There! Sending one back!");
                        let _ = web3.send_msg(MsgToWebView::HelloToWebview);
                    }
                    MsgFromWebView::OpenOptionalWindow => {
                        println!("Open the optional window!");
                        webopt.show(&event_loop_target, &proxy)
                    }
                },
            },
            _ => (),
        }
    });
}
