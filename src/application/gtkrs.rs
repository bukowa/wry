use crate::{
    application::{AppWebViewAttributes, AppWindowAttributes, ApplicationExt},
    ApplicationDispatcher, Callback, Icon, Message, Result, WebView, WebViewAttributes,
    WebViewBuilder, WebviewMessage, WindowMessage,
};

use std::{
    collections::HashMap,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use gio::{ApplicationExt as GioApplicationExt, Cancellable};
use gtk::{
    Application as GtkApp, ApplicationWindow, ApplicationWindowExt, GtkWindowExt, Inhibit,
    WidgetExt,
};

pub type WindowId = u32;

struct EventLoopProxy(Arc<Mutex<Sender<Message>>>);

impl Clone for EventLoopProxy {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[derive(Clone)]
pub struct AppDispatcher {
    proxy: EventLoopProxy,
}

impl ApplicationDispatcher for AppDispatcher {
    fn dispatch_message(&self, message: Message) -> Result<()> {
        self.proxy.0.lock().unwrap().send(message).unwrap();
        Ok(())
    }

    fn add_window(
        &self,
        attributes: WebViewAttributes,
        callbacks: Option<Vec<Callback>>,
    ) -> Result<WindowId> {
        let (sender, receiver): (Sender<WindowId>, Receiver<WindowId>) = channel();
        self.dispatch_message(Message::NewWindow(attributes, callbacks, sender))?;
        Ok(receiver.recv().unwrap())
    }
}

pub struct Application {
    webviews: HashMap<u32, WebView>,
    app: GtkApp,
    event_loop_proxy: EventLoopProxy,
    event_loop_proxy_rx: Receiver<Message>,
}

impl ApplicationExt for Application {
    type Id = u32;
    type Dispatcher = AppDispatcher;

    fn new() -> Result<Self> {
        let app = GtkApp::new(None, Default::default())?;
        let cancellable: Option<&Cancellable> = None;
        app.register(cancellable)?;
        app.activate();

        let (event_loop_proxy_tx, event_loop_proxy_rx) = channel();

        Ok(Self {
            webviews: HashMap::new(),
            app,
            event_loop_proxy: EventLoopProxy(Arc::new(Mutex::new(event_loop_proxy_tx))),
            event_loop_proxy_rx,
        })
    }

    fn create_webview(
        &mut self,
        attributes: WebViewAttributes,
        callbacks: Option<Vec<Callback>>,
    ) -> Result<Self::Id> {
        let (window_attrs, webview_attrs) = attributes.split();
        let window = _create_window(&self.app, window_attrs)?;
        let webview = _create_webview(window, webview_attrs, callbacks)?;
        let id = webview.window().get_id();
        self.webviews.insert(id, webview);

        Ok(id)
    }

    fn dispatcher(&self) -> Self::Dispatcher {
        AppDispatcher {
            proxy: self.event_loop_proxy.clone(),
        }
    }

    fn run(self) {
        let shared_webviews = Arc::new(Mutex::new(self.webviews));
        let shared_webviews_ = shared_webviews.clone();

        {
            let webviews = shared_webviews.lock().unwrap();
            for (id, w) in webviews.iter() {
                let shared_webviews_ = shared_webviews_.clone();
                let id_ = *id;
                w.window().connect_delete_event(move |_window, _event| {
                    shared_webviews_.lock().unwrap().remove(&id_);
                    Inhibit(false)
                });
            }
        }

        loop {
            {
                let webviews = shared_webviews.lock().unwrap();

                if webviews.is_empty() {
                    break;
                }

                for (_, w) in webviews.iter() {
                    let _ = w.evaluate_script();
                }
            }

            while let Ok(message) = self.event_loop_proxy_rx.try_recv() {
                match message {
                    Message::NewWindow(attributes, callbacks, sender) => {
                        let (window_attrs, webview_attrs) = attributes.split();
                        let window = _create_window(&self.app, window_attrs).unwrap();
                        sender.send(window.get_id()).unwrap();
                        let webview = _create_webview(window, webview_attrs, callbacks).unwrap();
                        let id = webview.window().get_id();
                        let shared_webviews_ = shared_webviews_.clone();
                        webview
                            .window()
                            .connect_delete_event(move |_window, _event| {
                                shared_webviews_.lock().unwrap().remove(&id);
                                Inhibit(false)
                            });
                        let mut webviews = shared_webviews.lock().unwrap();
                        webviews.insert(id, webview);
                    }
                    Message::Webview(id, webview_message) => {
                        if let Some(webview) = shared_webviews.lock().unwrap().get_mut(&id) {
                            match webview_message {
                                WebviewMessage::EvalScript(script) => {
                                    let _ = webview.dispatch_script(&script);
                                }
                            }
                        }
                    }
                    Message::Window(id, window_message) => {
                        if let Some(webview) = shared_webviews.lock().unwrap().get(&id) {
                            let window = webview.window();
                            match window_message {
                                WindowMessage::SetResizable(resizable) => {
                                    window.set_resizable(resizable);
                                }
                                WindowMessage::SetTitle(title) => window.set_title(&title),
                                WindowMessage::Maximize => {
                                    window.maximize();
                                }
                                WindowMessage::Unmaximize => {
                                    window.unmaximize();
                                }
                                WindowMessage::Minimize => {
                                    window.iconify();
                                }
                                WindowMessage::Unminimize => {
                                    window.deiconify();
                                }
                                WindowMessage::Show => {
                                    window.show();
                                }
                                WindowMessage::Hide => {
                                    window.hide();
                                }
                                WindowMessage::SetTransparent(_transparent) => {
                                    // TODO
                                }
                                WindowMessage::SetDecorations(decorations) => {
                                    window.set_decorated(decorations);
                                }
                                WindowMessage::SetAlwaysOnTop(always_on_top) => {
                                    window.set_keep_above(always_on_top);
                                }
                                WindowMessage::SetWidth(width) => {
                                    window.resize(width as i32, window.get_size().1);
                                }
                                WindowMessage::SetHeight(height) => {
                                    window.resize(window.get_size().0, height as i32);
                                }
                                WindowMessage::Resize { width, height } => {
                                    window.resize(width as i32, height as i32);
                                }
                                WindowMessage::SetMinSize {
                                    min_width,
                                    min_height,
                                } => {
                                    window.set_geometry_hints::<ApplicationWindow>(
                                        None,
                                        Some(&gdk::Geometry {
                                            min_width: min_width as i32,
                                            min_height: min_height as i32,
                                            max_width: 0,
                                            max_height: 0,
                                            base_width: 0,
                                            base_height: 0,
                                            width_inc: 0,
                                            height_inc: 0,
                                            min_aspect: 0f64,
                                            max_aspect: 0f64,
                                            win_gravity: gdk::Gravity::Center,
                                        }),
                                        gdk::WindowHints::MIN_SIZE,
                                    );
                                }
                                WindowMessage::SetMaxSize {
                                    max_width,
                                    max_height,
                                } => {
                                    window.set_geometry_hints::<ApplicationWindow>(
                                        None,
                                        Some(&gdk::Geometry {
                                            min_width: 0,
                                            min_height: 0,
                                            max_width: max_width as i32,
                                            max_height: max_height as i32,
                                            base_width: 0,
                                            base_height: 0,
                                            width_inc: 0,
                                            height_inc: 0,
                                            min_aspect: 0f64,
                                            max_aspect: 0f64,
                                            win_gravity: gdk::Gravity::Center,
                                        }),
                                        gdk::WindowHints::MAX_SIZE,
                                    );
                                }
                                WindowMessage::SetX(x) => {
                                    let (_, y) = window.get_position();
                                    window.move_(x as i32, y);
                                }
                                WindowMessage::SetY(y) => {
                                    let (x, _) = window.get_position();
                                    window.move_(x, y as i32);
                                }
                                WindowMessage::SetPosition { x, y } => {
                                    window.move_(x as i32, y as i32);
                                }
                                WindowMessage::SetFullscreen(fullscreen) => {
                                    if fullscreen {
                                        window.fullscreen();
                                    } else {
                                        window.unfullscreen();
                                    }
                                }
                                WindowMessage::SetIcon(icon) => {
                                    if let Ok(icon) = load_icon(icon) {
                                        window.set_icon(Some(&icon));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            gtk::main_iteration();
        }
    }
}

fn load_icon(icon: Icon) -> Result<gdk_pixbuf::Pixbuf> {
    let image = image::load_from_memory(&icon.0)?.into_rgba8();
    let (width, height) = image.dimensions();
    let row_stride = image.sample_layout().height_stride;
    Ok(gdk_pixbuf::Pixbuf::from_mut_slice(
        image.into_raw(),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width as i32,
        height as i32,
        row_stride as i32,
    ))
}

fn _create_window(app: &GtkApp, attributes: AppWindowAttributes) -> Result<ApplicationWindow> {
    //TODO window config (missing transparent)
    let window = ApplicationWindow::new(app);

    window.set_geometry_hints::<ApplicationWindow>(
        None,
        Some(&gdk::Geometry {
            min_width: attributes.min_width.unwrap_or_default() as i32,
            min_height: attributes.min_height.unwrap_or_default() as i32,
            max_width: attributes.max_width.unwrap_or_default() as i32,
            max_height: attributes.max_height.unwrap_or_default() as i32,
            base_width: 0,
            base_height: 0,
            width_inc: 0,
            height_inc: 0,
            min_aspect: 0f64,
            max_aspect: 0f64,
            win_gravity: gdk::Gravity::Center,
        }),
        (if attributes.min_width.is_some() || attributes.min_height.is_some() {
            gdk::WindowHints::MIN_SIZE
        } else {
            gdk::WindowHints::empty()
        }) | (if attributes.max_width.is_some() || attributes.max_height.is_some() {
            gdk::WindowHints::MAX_SIZE
        } else {
            gdk::WindowHints::empty()
        }),
    );

    if attributes.resizable {
        window.set_default_size(attributes.width as i32, attributes.height as i32);
    } else {
        window.set_size_request(attributes.width as i32, attributes.height as i32);
    }

    window.set_skip_taskbar_hint(attributes.skip_taskbar);
    window.set_resizable(attributes.resizable);
    window.set_title(&attributes.title);
    if attributes.maximized {
        window.maximize();
    }
    window.set_visible(attributes.visible);
    window.set_decorated(attributes.decorations);
    window.set_keep_above(attributes.always_on_top);

    match (attributes.x, attributes.y) {
        (Some(x), Some(y)) => window.move_(x as i32, y as i32),
        _ => {}
    }

    if attributes.fullscreen {
        window.fullscreen();
    }
    if let Some(icon) = attributes.icon {
        window.set_icon(Some(&load_icon(icon)?));
    }

    Ok(window)
}

fn _create_webview(
    window: ApplicationWindow,
    attributes: AppWebViewAttributes,
    callbacks: Option<Vec<Callback>>,
) -> Result<WebView> {
    let mut webview = WebViewBuilder::new(window)?;
    for js in attributes.initialization_scripts {
        webview = webview.initialize_script(&js);
    }
    if let Some(cbs) = callbacks {
        for Callback { name, function } in cbs {
            webview = webview.add_callback(&name, function);
        }
    }
    webview = match attributes.url {
        Some(url) => webview.load_url(&url)?,
        None => webview,
    };

    let webview = webview.build()?;
    Ok(webview)
}
