use std::{cell::RefCell, collections::HashMap, net::Ipv4Addr, rc::Rc};
use shared::{BackEndRequest, pipuck::{Descriptor, Request, Update}};
use web_sys::HtmlInputElement;
use yew::{prelude::*, web_sys::HtmlTextAreaElement};

enum RaspberryPi {
    Connected {
        addr: Ipv4Addr,
        battery: Result<i32, String>,
        signal: Result<i32, String>,
        terminal: String,
    },
    Disconnected,
}

pub struct Instance {
    pub descriptor: Descriptor,
    pub optitrack_pos: [f32; 3],
    rpi: RaspberryPi,
    camera_stream: HashMap<String, Result<String, String>>,
}

// a lot of stuff here seems like it should be implemented directly on the component,
// update for instance would be cleaner if it was implemented via a ComponentLink<T>
impl Instance {
    pub fn new(descriptor: Descriptor) -> Self {
        Self { 
            descriptor, 
            optitrack_pos: [0.0, 0.0, 0.0],
            rpi: RaspberryPi::Disconnected,
            camera_stream: Default::default(),
        }
    }

    pub fn update(&mut self, update: Update) {
        match update {
            Update::Battery(reading) => if let RaspberryPi::Connected { battery, ..} = &mut self.rpi {
                *battery = Ok(reading);
            },
            Update::Camera { camera, result } => {
                self.camera_stream
                    .insert(camera, result
                        .map(|bytes| base64::encode(bytes)));
            },
            Update::FernbedienungConnected(addr) => 
                self.rpi = RaspberryPi::Connected {
                    addr,
                    battery: Err(String::from("Unknown")),
                    signal: Err(String::from("Unknown")),
                    terminal: Default::default(),
                },
            Update::FernbedienungDisconnected => 
                self.rpi = RaspberryPi::Disconnected,
            Update::FernbedienungSignal(strength) => {
                if let RaspberryPi::Connected { signal, ..} = &mut self.rpi {
                    *signal = Ok(strength);
                }
            },
            Update::Bash(response) => if let RaspberryPi::Connected { terminal, ..} = &mut self.rpi {
                terminal.push_str(&response);
            },
        }
    }
}

pub struct Card {
    link: ComponentLink<Self>,
    props: Props,
    bash_terminal_visible: bool,
    bash_textarea: NodeRef,
    bash_input: NodeRef,
    camera_dialog_active: bool,
}

#[derive(Clone, Properties)]
pub struct Props {
    pub instance: Rc<RefCell<Instance>>,
    pub parent: ComponentLink<crate::UserInterface>,
}

pub enum Msg {
    ToggleBashTerminal,
    ToggleCameraStream,
    SendBashCommand,
}

// is it possible to just add a callback to the update method
impl Component for Card {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {
        // if props contains a closure, I could use that to communicate with the actual instance
        Card { 
            props,
            link,
            bash_terminal_visible: false,
            bash_textarea: NodeRef::default(),
            bash_input: NodeRef::default(),
            camera_dialog_active: false
        }
    }


    fn rendered(&mut self, _: bool) {
        if let Some(textarea) = self.bash_textarea.cast::<HtmlTextAreaElement>() {
            textarea.set_scroll_top(textarea.scroll_height());
        }
    }


    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        let mut pipuck = self.props.instance.borrow_mut();
        match msg {
            Msg::SendBashCommand => match self.bash_input.cast::<HtmlInputElement>() {
                Some(input) => {
                    let pipuck_request = Request::BashTerminalRun(input.value());
                    input.set_value("");
                    let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
                    self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                    true
                },
                _ => false
            },
            Msg::ToggleBashTerminal => {
                match self.bash_terminal_visible {
                    false => {
                        if let RaspberryPi::Connected { terminal, .. } = &mut pipuck.rpi {
                            terminal.clear();
                        }
                        let pipuck_request = Request::BashTerminalStart;
                        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.bash_terminal_visible = true;
                    },
                    true => {
                        let pipuck_request = Request::BashTerminalStop;
                        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.bash_terminal_visible = false;
                    }
                }
                true
            },
            Msg::ToggleCameraStream => {
                match self.camera_dialog_active {
                    false => {
                        let pipuck_request = Request::CameraStreamEnable(true);
                        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        pipuck.camera_stream.clear();
                        self.camera_dialog_active = true;
                    },
                    true => {
                        let pipuck_request = Request::CameraStreamEnable(false);
                        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.camera_dialog_active = false;
                    }
                }
                true
            },
        }
    }

    // this fires when the parent changes the properties of this component
    fn change(&mut self, _: Self::Properties) -> ShouldRender {
        true
    }

    fn view(&self) -> Html {
        let pipuck = self.props.instance.borrow();
        let (batt_level, batt_info) = match &pipuck.rpi {
            RaspberryPi::Disconnected => (0, String::from("Unknown")),
            RaspberryPi::Connected { battery, .. } => match battery {
                Err(message) => (0, message.clone()),
                Ok(level) => (match level {
                    0..=24 => 1,
                    25..=49 => 2,
                    50..=74 => 3,
                    _ => 4,
                }, format!("{}%", level))
            }
        };

        html! {
            <div class="card">
                <header class="card-header">
                    <nav class="card-header-title is-shadowless has-background-white-ter level is-mobile">
                        <div class="level-left">
                            <p class="level-item subtitle is-size-4">{ &pipuck.descriptor.id }</p>
                        </div>
                        <div class="level-right">
                            <figure class="level-item image mx-0 is-48x48">
                                <img src=format!("images/batt{}.svg", batt_level) title=batt_info/>
                            </figure>
                        </div>
                    </nav>
                </header>
                <div class="card-content">
                    <div class="content">
                        { self.render_rpi(&pipuck) }
                        { self.render_identifiers(&pipuck) }
                    </div>
                </div>
                { self.render_menu(&pipuck) }
                { self.render_camera_modal(&pipuck) }
            </div>
        }
    }
}

impl Card {
    fn render_camera_modal(&self, pipuck: &Instance) -> Html {
        if self.camera_dialog_active {
            let disable_onclick = self.link.callback(|_| Msg::ToggleCameraStream);
            html! {
                <div class="modal is-active">
                    <div class="modal-background" onclick=disable_onclick />
                    <div style="width:50%" class="modal-content">
                        <div class="container is-clipped">
                            <div class="columns is-multiline is-mobile"> { 
                                pipuck.camera_stream.iter().map(|(id, result)| match result {
                                    Ok(encoded) => html! {
                                        <div class="column is-half">
                                            <figure class="image">
                                                <img src=format!("data:image/jpeg;base64,{}", encoded) />
                                                <figcaption class="has-text-grey-lighter"> { &id } </figcaption>
                                            </figure>
                                        </div>
                                    },
                                    Err(error) => html! {
                                        <div class="column is-half">
                                            <figure class="image">
                                                <p class="has-text-white"> { error.clone () }</p>
                                                <figcaption class="has-text-grey-lighter"> { &id } </figcaption>
                                            </figure>
                                        </div>
                                    }
                                }).collect::<Html>()
                            } </div>
                        </div>
                    </div>
                </div>
            }
        }
        else {
            html! {}
        }
    }

    fn render_rpi(&self, pipuck: &Instance) -> Html {
        let (wifi_signal_level, wifi_signal_info) = match &pipuck.rpi {
            RaspberryPi::Disconnected => (0, String::from("Disconnected")),
            RaspberryPi::Connected { signal, .. } => match signal {
                Err(message) => (0, message.clone()),
                Ok(level) => (match level + 90 {
                    0..=24 => 1,
                    25..=49 => 2,
                    50..=74 => 3,
                    _ => 4,
                }, format!("{}%", level + 90))
            }
        };
        let (term_disabled, term_content) = match &pipuck.rpi {
            RaspberryPi::Disconnected => (true, String::new()),
            RaspberryPi::Connected { terminal, ..} => (false, terminal.clone())
        };
        let mut term_classes = classes!("column", "is-full");
        if !self.bash_terminal_visible {
            term_classes.push("is-hidden");
        }
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleBashTerminal);
        let term_onkeydown = self.link.batch_callback(|event: KeyboardEvent| match event.key().as_ref() {
            "Enter" => Some(Msg::SendBashCommand),
            _ => None,
        });
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Raspberry Pi" }</p>
                    </div>
                    <div class="level-right">
                        <button class="level-item button" onclick=term_btn_onclick disabled=term_disabled> {
                            if self.bash_terminal_visible {
                                "Close Bash terminal"
                            }
                            else {
                                "Open Bash terminal"
                            }
                        } </button>
                    </div>
                </nav>
                
                <div class="columns is-multiline is-mobile">
                    <div class=term_classes>
                        <div>
                            <div class="field">
                                <div class="control">
                                    <textarea ref=self.bash_textarea.clone()
                                              class="textarea is-family-monospace"
                                              readonly=false>
                                              { term_content }
                                    </textarea>
                                </div>
                            </div>
                            <div class="field">
                                <div class="control">
                                    <input ref=self.bash_input.clone()
                                           class="input is-family-monospace"
                                           type="text" 
                                           disabled=term_disabled
                                           placeholder="Type a command and press enter"
                                           onkeydown=term_onkeydown />
                                </div>
                            </div>
                        </div>
                    </div> 
                    
                    <div class="column is-two-fifths">
                        <div class="notification has-text-centered">
                            <p style="line-height:32px"> {
                                pipuck.descriptor.rpi_macaddr.to_string()
                            } </p>
                        </div>
                    </div>
                    <div class="column is-two-fifths">
                        <div class="notification has-text-centered">
                            <p style="line-height:32px"> {
                                match pipuck.rpi {
                                    RaspberryPi::Connected { addr, .. } => addr.to_string(),
                                    RaspberryPi::Disconnected => "Disconnected".to_owned()
                                }
                            } </p>
                        </div>
                    </div>
                    <div class="column is-one-fifth">
                        <div class="notification has-text-centered">
                            <figure class="image mx-auto is-32x32">
                                <img src=format!("images/wifi{}.svg", wifi_signal_level) title=wifi_signal_info />
                            </figure>
                        </div>
                    </div>
                </div>
            </>
        }
    }
    
    fn render_identifiers(&self, pipuck: &Instance) -> Html {
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Optitrack" }</p>
                    </div>    
                </nav>
                <div class="columns is-multiline is-mobile">
                    <div class="column is-one-fifth">
                        <div class="notification has-text-centered">
                            <p style="line-height:32px"> {
                                pipuck.descriptor.optitrack_id
                                    .map_or_else(|| "-".to_owned(), |id| id.to_string())
                            } </p>
                        </div>
                    </div>
                    <div class="column is-four-fifths">
                        <div class="notification">
                            <nav class="level is-mobile"> {
                                pipuck.optitrack_pos.iter().map(|coord| html! {
                                    <p style="line-height:32px" class="level-item">{ format!("{:.3}", coord) }</p>
                                }).collect::<Html>()  
                            } </nav>
                        </div>
                    </div>
                </div>
            </>
        }
    }

    fn render_menu(&self, pipuck: &Instance) -> Html {
        let toggle_camera_stream_onclick = self.link.callback(|_| Msg::ToggleCameraStream);

        let pipuck_request = Request::RaspberryPiReboot;
        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
        let reboot_rpi_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let pipuck_request = Request::RaspberryPiHalt;
        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
        let halt_rpi_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let pipuck_request = Request::Identify;
        let request = BackEndRequest::PiPuckRequest(pipuck.descriptor.id.clone(), pipuck_request);
        let identify_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        html! {
            <footer class="card-footer">
                {
                    match pipuck.rpi {
                        RaspberryPi::Connected {..} => html! {
                            <>
                                <a class="card-footer-item" onclick=toggle_camera_stream_onclick>{ "Show cameras" }</a>
                                <a class="card-footer-item" onclick=identify_onclick>{ "Identify" }</a>
                            </>
                        },
                        RaspberryPi::Disconnected => html! {
                            <>
                                <p class="card-footer-item has-text-grey-light">{ "Show cameras" }</p>
                                <p class="card-footer-item has-text-grey-light">{ "Identify" }</p>
                            </>
                        },
                    }
                }
                <div class="card-footer-item dropdown is-hoverable">
                    <div class="dropdown-trigger">
                        <a>
                            <span>{ "Raspberry Pi" }</span>
                            <span class="icon is-small">
                                <i class="mdi mdi-menu-down" />
                            </span>
                        </a>
                    </div>
                    <div class="dropdown-menu" id="dropdown-menu" role="menu">
                        <div class="dropdown-content"> {
                            match pipuck.rpi {
                                RaspberryPi::Connected {..} => html! {
                                    <a class="dropdown-item" onclick=halt_rpi_onclick>{ "Halt" }</a>
                                },
                                RaspberryPi::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Halt" }</p>
                                },
                            }
                        } {
                            match pipuck.rpi {
                                RaspberryPi::Connected {..} => html! {
                                    <a class="dropdown-item" onclick=reboot_rpi_onclick>{ "Reboot" }</a>
                                },
                                RaspberryPi::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Reboot" }</p>
                                },
                            }
                        } </div>
                    </div>
                </div>
            </footer>
        }
    }
}
