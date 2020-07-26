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
    um::winuser::{GetClientRect, SetForegroundWindow},
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

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum ShowWebview {
    Immediately,
    OnNavigationCompleted,
    OnContentLoading,
}

impl<T: 'static> ReceiveWebviewMessage<T> for NoMsg {
    fn pass_to_event_loop_proxy(self: Self, _: &EventLoopProxy<T>) {}
}

#[derive(Debug)]
pub enum Error {
    ControllerNotCreated,
    WebviewNotShown,
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

#[derive(Clone)]
pub struct WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView>
where
    EventLoopType: 'static + Clone,
    MsgToWebView: Debug + Serialize + 'static + Clone,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static + Clone,
{
    event_loop_type: PhantomData<EventLoopType>,
    msg_to_webview: PhantomData<MsgToWebView>,
    msg_from_webview: PhantomData<MsgFromWebView>,
    window_builder: Option<WindowBuilder>,
    show_on: ShowWebview,
    #[allow(clippy::type_complexity)]
    // settings_fn: Option<Box<dyn Fn(&Settings) -> Result<(), webview2::Error>>>,
    settings_fn: Option<fn(&Settings) -> Result<(), webview2::Error>>,
    #[allow(clippy::type_complexity)]
    // webview_fn: Option<Box<dyn Fn(&webview2::WebView) -> Result<(), webview2::Error>>>,
    webview_fn: Option<fn(&webview2::WebView) -> Result<(), webview2::Error>>,
}

impl<EventLoopType> WebViewBuilder<EventLoopType, NoMsg, NoMsg>
where
    EventLoopType: 'static + Clone,
{
    #[allow(clippy::new_without_default)]
    pub fn new() -> WebViewBuilder<EventLoopType, NoMsg, NoMsg> {
        WebViewBuilder {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: None,
            show_on: ShowWebview::OnNavigationCompleted,
            webview_fn: None,
            settings_fn: None,
        }
    }
}

impl<EventLoopType, MsgToWebView, MsgFromWebView>
    WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView>
where
    EventLoopType: 'static + Clone,
    MsgToWebView: Debug + Serialize + 'static + Clone,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static + Clone,
{
    pub fn msg_from_webview<
        T: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static + Clone,
    >(
        self,
    ) -> WebViewBuilder<EventLoopType, MsgToWebView, T> {
        WebViewBuilder::<EventLoopType, MsgToWebView, T> {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: self.window_builder,
            show_on: self.show_on,
            webview_fn: self.webview_fn,
            settings_fn: self.settings_fn,
        }
    }
    pub fn msg_to_webview<T: Debug + Serialize + 'static + Clone>(
        self,
    ) -> WebViewBuilder<EventLoopType, T, MsgFromWebView> {
        WebViewBuilder::<EventLoopType, T, MsgFromWebView> {
            event_loop_type: PhantomData,
            msg_to_webview: PhantomData,
            msg_from_webview: PhantomData,
            window_builder: self.window_builder,
            show_on: self.show_on,
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
        settings_closure: fn(&Settings) -> Result<(), webview2::Error>,
    ) -> Self {
        self.settings_fn = Some(settings_closure);
        self
    }

    /// Delay the showing until the webview controller responds
    pub fn show_on(mut self, show_on: ShowWebview) -> Self {
        self.show_on = show_on;
        self
    }

    /// Webview init closure
    pub fn webview_init(
        mut self,
        webview_closure: fn(&webview2::WebView) -> Result<(), webview2::Error>,
    ) -> Self {
        self.webview_fn = Some(webview_closure);
        self
    }

    /// Tries to build the webview
    pub fn build(
        &self,
        event_loop: &EventLoop<EventLoopType>,
    ) -> Result<WebViewWrapper<MsgToWebView>, Error> {
        let proxy = event_loop.create_proxy();
        self.build_with_proxy(event_loop, &proxy)
    }

    /// Tries to build the webview
    pub fn build_with_proxy(
        &self,
        event_loop: &EventLoopWindowTarget<EventLoopType>,
        event_loop_proxy: &EventLoopProxy<EventLoopType>,
    ) -> Result<WebViewWrapper<MsgToWebView>, Error> {
        let window = self
            .window_builder
            .clone()
            .unwrap_or_else(|| WindowBuilder::new().with_title(""))
            .with_visible(self.show_on == ShowWebview::Immediately)
            .build(&event_loop)?;
        let parent_hwnd = window.hwnd() as u32;
        let window_ref = Rc::new(window);
        let webview = WebViewWrapper {
            msg_to_webview_type: PhantomData::<MsgToWebView>,
            controller: Rc::new(RefCell::new(None)),
            window: window_ref.clone(),
        };
        let settings = self.settings_fn;
        let webview_with = self.webview_fn;
        let controller_weak = Rc::downgrade(&webview.controller);
        let window_weak = Rc::downgrade(&window_ref);
        let event_loop_proxy = event_loop_proxy.clone();
        let show_on = self.show_on;

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
                        window_rc.request_redraw();
                    }
                    Ok(())
                })?;

                // Show the window after event trigger
                let window_weak_ = window_weak.clone();
                let controller_weak_ = controller_weak.clone();
                let do_it = move || {
                    if let Some(controller_rc) = controller_weak_.upgrade() {
                        if let Some(controller) = controller_rc.borrow().as_ref() {
                            controller.put_is_visible(true)?;
                        }
                    }
                    if let Some(_window_rc) = window_weak_.upgrade() {
                        _window_rc.set_visible(true);
                    }
                    Ok(())
                };
                match show_on {
                    ShowWebview::Immediately => {}
                    ShowWebview::OnNavigationCompleted => {
                        webview.add_navigation_completed(move |_, _args| do_it())?;
                    }
                    ShowWebview::OnContentLoading => {
                        webview.add_content_loading(move |_, _args| do_it())?;
                    }
                }

                // Webview requested a close?
                let window_weak_ = window_weak.clone();
                webview.add_window_close_requested(move |_webview| {
                    if let Some(_window_rc) = window_weak_.upgrade() {
                        // TODO: Send message to eventloop?
                    }
                    Ok(())
                })?;

                // Message passing
                webview.add_web_message_received(move |_webview, args| {
                    let message = args.try_get_web_message_as_string()?;

                    match serde_json::from_str::<MsgFromWebView>(&message) {
                        Ok(msg) => msg.pass_to_event_loop_proxy(&event_loop_proxy),
                        Err(_err) => {
                            // TODO: Should we send parsing error message to event_loop_proxy?
                            #[cfg(debug_assertions)]
                            println!(
                                "Webview gave unparseable result: {:?}, error: {:?}",
                                message, _err
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

    /// Build optional window
    ///
    /// This window does not exist until it's shown, after closing it, it needs
    /// to be shown again.
    pub fn build_optional(
        &self,
        _event_loop: &EventLoop<EventLoopType>,
    ) -> WebViewOptional<EventLoopType, MsgToWebView, MsgFromWebView> {
        WebViewOptional::new(self.clone())
    }
}

pub struct WebViewWrapper<MsgToWebView>
where
    MsgToWebView: Serialize + 'static,
{
    msg_to_webview_type: PhantomData<MsgToWebView>,

    // Controller persists the webview, while it's alive, the webview is shown
    controller: Rc<RefCell<Option<webview2::Controller>>>,
    window: Rc<Window>,
}

impl<MsgToWebView> WebViewWrapper<MsgToWebView>
where
    MsgToWebView: Debug + Serialize + 'static + Clone,
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

    /// Call the webview instance
    pub fn webview_with(&self, mut cb: impl FnMut(&webview2::WebView)) -> Result<(), Error> {
        let c = self.controller.borrow_mut();
        if let Some(controller) = c.as_ref() {
            let mut webview = controller.get_webview()?;
            cb(&mut webview);
            Ok(())
        } else {
            Err(Error::ControllerNotCreated)
        }
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

pub struct WebViewOptional<EventLoopType, MsgToWebView, MsgFromWebView>
where
    EventLoopType: 'static + Clone,
    MsgToWebView: Debug + Serialize + 'static + Clone,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static + Clone,
{
    // builder: Box<Fn() -> WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView> + 'static>,
    builder: WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView>,
    instance: Rc<RefCell<Option<WebViewWrapper<MsgToWebView>>>>,
}

impl<EventLoopType, MsgToWebView, MsgFromWebView>
    WebViewOptional<EventLoopType, MsgToWebView, MsgFromWebView>
where
    EventLoopType: 'static + Clone,
    MsgToWebView: Debug + Serialize + 'static + Clone,
    MsgFromWebView: ReceiveWebviewMessage<EventLoopType> + DeserializeOwned + 'static + Clone,
{
    pub(crate) fn new(
        builder: WebViewBuilder<EventLoopType, MsgToWebView, MsgFromWebView>,
    ) -> Self {
        WebViewOptional {
            builder,
            instance: Rc::new(RefCell::new(None)),
        }
    }
    /// Pass message to the WebView
    pub fn send_msg(&self, m: MsgToWebView) -> Result<(), Error> {
        let value = self.instance.borrow();
        if let Some(value) = value.as_ref() {
            value.send_msg(m)
        } else {
            Err(Error::WebviewNotShown)
        }
    }

    /// Is matching window?
    pub fn is_window(&self, window_id: &WindowId) -> bool {
        let value = self.instance.borrow();
        if let Some(value) = value.as_ref() {
            value.is_window(window_id)
        } else {
            false
        }
    }

    /// Call the webview instance
    pub fn webview_with(&self, cb: impl FnMut(&webview2::WebView)) -> Result<(), Error> {
        let value = self.instance.borrow();
        if let Some(value) = value.as_ref() {
            value.webview_with(cb)
        } else {
            Err(Error::WebviewNotShown)
        }
    }

    pub fn show(
        &mut self,
        event_loop: &EventLoopWindowTarget<EventLoopType>,
        proxy: &EventLoopProxy<EventLoopType>,
    ) {
        let mut value = self.instance.borrow_mut();
        match value.as_ref() {
            Some(instance) => {
                // How come winit does not have setting focus action? I noticed
                // that winapi call SetFocus does not work always, but instead
                // SetForegroundWindow did work.
                unsafe { SetForegroundWindow(instance.window.hwnd() as HWND) };
            }
            None => {
                let builder = self.builder.clone();
                *value = Some(builder.build_with_proxy(event_loop, proxy).unwrap());
            }
        }
    }

    pub fn handle_window_event(
        &mut self,
        event: &WindowEvent,
        window_id: &WindowId,
    ) -> Result<(), Error> {
        let mut value = self.instance.borrow_mut();
        if let Some(instance) = value.as_ref() {
            if instance.is_window(window_id) {
                if let WindowEvent::CloseRequested = event {
                    *value = None;
                    return Ok(());
                }
            }
            instance.handle_window_event(&event, window_id)
        } else {
            Err(Error::WebviewNotShown)
        }
    }
}
