//! This is a msedge webview2 builder, see the main.rs for usage.
//!
//! Intention here is to provide simple builder that creates the winit window
//! and inits the msedge webview2.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::cell::RefCell;
use std::mem;
use std::{fmt::Debug, marker::PhantomData, rc::Rc};
use webview2::Settings;
use winapi::{
    shared::windef::{HWND, RECT},
    um::winuser::GetClientRect,
};
use winit::event::WindowEvent;
use winit::platform::windows::WindowExtWindows;
use winit::{
    error::OsError,
    event_loop::{EventLoop, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};

#[derive(Copy, Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum NoMsg {}

impl<T: 'static> ReceiveWebviewMessage<T> for NoMsg {
    fn pass_to_event_loop_proxy(self: Self, _: &EventLoopProxy<T>) {}
}

#[derive(Debug)]
pub enum Error {
    ControllerNotCreated,
    SerializationError(serde_json::Error),
    WebView2Error(webview2::Error),
    WindowBuildError(OsError),
}

impl From<webview2::Error> for Error {
    fn from(er: webview2::Error) -> Self {
        Error::WebView2Error(er)
    }
}

impl From<OsError> for Error {
    fn from(er: OsError) -> Self {
        Error::WindowBuildError(er)
    }
}

impl From<serde_json::Error> for Error {
    fn from(er: serde_json::Error) -> Self {
        Error::SerializationError(er)
    }
}

pub trait ReceiveWebviewMessage<T: 'static> {
    fn pass_to_event_loop_proxy(self: Self, proxy: &EventLoopProxy<T>);
}

pub struct WebViewBuilder<EventLoopType = (), MsgToWebView = NoMsg, MsgFromWebView = NoMsg>
where
    EventLoopType: 'static,
    MsgToWebView: Debug + Serialize + 'static,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static,
{
    event_loop_type: PhantomData<EventLoopType>,
    msg_to_webview: PhantomData<MsgToWebView>,
    msg_from_webview: PhantomData<MsgFromWebView>,
    window_builder: Option<WindowBuilder>,
    #[allow(clippy::type_complexity)]
    settings_fn: Option<Box<dyn FnOnce(&Settings) -> Result<(), webview2::Error>>>,
    #[allow(clippy::type_complexity)]
    webview_fn: Option<Box<dyn FnOnce(&webview2::WebView) -> Result<(), webview2::Error>>>,
}

impl<EventLoopType> WebViewBuilder<EventLoopType> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> WebViewBuilder<EventLoopType, NoMsg, NoMsg> {
        WebViewBuilder {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: None,
            webview_fn: None,
            settings_fn: None,
        }
    }
}

impl<EventLoopType, MsgToWebView, MsgFromWebView>
    WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView>
where
    EventLoopType: 'static,
    MsgToWebView: Debug + Serialize + 'static,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static,
{
    pub fn msg_from_webview<
        T: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static,
    >(
        self,
    ) -> WebViewBuilder<EventLoopType, MsgToWebView, T> {
        WebViewBuilder::<EventLoopType, MsgToWebView, T> {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: self.window_builder,
            webview_fn: self.webview_fn,
            settings_fn: self.settings_fn,
        }
    }
    pub fn msg_to_webview<T: Debug + Serialize + 'static>(
        self,
    ) -> WebViewBuilder<EventLoopType, T, MsgFromWebView> {
        WebViewBuilder::<EventLoopType, T, MsgFromWebView> {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: self.window_builder,
            webview_fn: self.webview_fn,
            settings_fn: self.settings_fn,
        }
    }

    pub fn window_builder(mut self, window_builder: WindowBuilder) -> Self {
        self.window_builder = Some(window_builder);
        self
    }

    /// Settings init closure
    pub fn settings(
        mut self,
        settings_closure: impl FnOnce(&Settings) -> Result<(), webview2::Error> + 'static,
    ) -> Self {
        self.settings_fn = Some(Box::new(settings_closure));
        self
    }

    /// Webview init closure
    pub fn webview_init(
        mut self,
        webview_closure: impl FnOnce(&webview2::WebView) -> Result<(), webview2::Error> + 'static,
    ) -> Self {
        self.webview_fn = Some(Box::new(webview_closure));
        self
    }

    /// Tries to build the webview
    pub fn build(
        self,
        event_loop: &EventLoop<EventLoopType>,
    ) -> Result<WebView<MsgToWebView>, Error> {
        let proxy = event_loop.create_proxy();
        self.build_with_proxy(event_loop, &proxy)
    }

    /// Tries to build the webview
    pub fn build_with_proxy(
        self,
        event_loop: &EventLoopWindowTarget<EventLoopType>,
        event_loop_proxy: &EventLoopProxy<EventLoopType>,
    ) -> Result<WebView<MsgToWebView>, Error> {
        let window = self
            .window_builder
            .unwrap_or_else(|| WindowBuilder::new().with_title(""))
            .build(&event_loop)?;
        let parent_hwnd = window.hwnd() as u32;
        let window_ref = Rc::new(window);
        let webview = WebView {
            msg_to_webview_type: PhantomData::<MsgToWebView>,
            controller: Rc::new(RefCell::new(None)),
            window: window_ref.clone(),
        };
        let settings = self.settings_fn;
        let webview_with = self.webview_fn;
        let controller_weak = Rc::downgrade(&webview.controller);
        let window_weak = Rc::downgrade(&window_ref);
        let event_loop_proxy = event_loop_proxy.clone();

        webview2::EnvironmentBuilder::new().build(move |env| {
            // Following is ran asynchronously somewhere after the
            // WebViewBuilder::build() finishes, for this reason the moved
            // variables must be passed as a weak.
            env?.create_controller(parent_hwnd as HWND, move |host| {
                let controller = host?;
                let webview = controller.get_webview()?;

                if let Some(settings_fn) = settings {
                    webview.get_settings().map(|o| settings_fn(&o))??;
                }

                unsafe {
                    let mut rect = mem::zeroed();
                    GetClientRect(parent_hwnd as HWND, &mut rect);
                    controller.put_bounds(rect)?;
                }

                let window_weak_ = window_weak.clone();
                webview.add_document_title_changed(move |args| {
                    if let Some(window_rc) = window_weak_.upgrade() {
                        let title = args.get_document_title()?;
                        window_rc.set_title(&title);
                    }
                    Ok(())
                })?;

                let window_weak_ = window_weak.clone();
                webview.add_content_loading(move |_, _args| {
                    if let Some(_window_rc) = window_weak_.upgrade() {
                        // window_rc.set_title("Loading ...");
                        // TODO: Send message to eventloop?
                    }
                    Ok(())
                })?;

                let window_weak_ = window_weak.clone();
                webview.add_window_close_requested(move |_webview| {
                    if let Some(_window_rc) = window_weak_.upgrade() {
                        // ...?
                        // TODO: Send message to eventloop?
                    }
                    Ok(())
                })?;

                webview.add_web_message_received(move |_webview, args| {
                    let message = args.try_get_web_message_as_string()?;
                    match serde_json::from_str::<MsgFromWebView>(&message) {
                        Ok(msg) => msg.pass_to_event_loop_proxy(&event_loop_proxy),
                        Err(err) => {
                            // TODO: Should we send parsing error message to event_loop_proxy?
                            #[cfg(debug_assertions)]
                            println!(
                                "Webview gave unparseable result: {:?}, error: {:?}",
                                message, err
                            );
                        }
                    }

                    Ok(())
                })?;

                if let Some(webview_with_fn) = webview_with {
                    webview_with_fn(&webview)?;
                }

                if let Some(controller_rc) = controller_weak.upgrade() {
                    let mut controller_cell = controller_rc.borrow_mut();
                    *controller_cell = Some(controller);
                }

                Ok(())
            })
        })?;
        Ok(webview)
    }
}

pub struct WebView<MsgToWebView>
where
    MsgToWebView: Serialize + 'static,
{
    msg_to_webview_type: PhantomData<MsgToWebView>,
    controller: Rc<RefCell<Option<webview2::Controller>>>,
    window: Rc<Window>,
}

impl<MsgToWebView> WebView<MsgToWebView>
where
    MsgToWebView: Debug + Serialize + 'static,
{
    /// Pass message to the WebView
    pub fn send_msg(&self, m: MsgToWebView) -> Result<(), Error> {
        let c = self.controller.borrow_mut();
        if let Some(controller) = c.as_ref() {
            let webview = controller.get_webview()?;
            let msgstr = &serde_json::to_string(&m)?;
            webview.post_web_message_as_json(msgstr)?;
        }
        Ok(())
    }

    /// Is matching window?
    pub fn is_window(&self, window_id: &WindowId) -> bool {
        window_id == &self.window.id()
    }

    /// Handle Window Event
    ///
    /// Runs the side effects to keep the webview2 control happy, you must plug
    /// this in to the main event loop.
    pub fn handle_window_event(&self, t: &WindowEvent, window_id: &WindowId) -> Result<(), Error> {
        if !self.is_window(window_id) {
            return Ok(());
        }
        let controller_maybe = self.controller.borrow_mut();
        let controller = controller_maybe
            .as_ref()
            .ok_or(Error::ControllerNotCreated)?;

        match t {
            WindowEvent::Moved(_) => {
                controller.notify_parent_window_position_changed()?;
            }

            WindowEvent::Resized(new_size) => {
                let r = RECT {
                    left: 0,
                    top: 0,
                    right: new_size.width as i32,
                    bottom: new_size.height as i32,
                };
                controller.put_bounds(r)?;
            }
            _ => (),
        };
        Ok(())
    }
}
