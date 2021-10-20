use std::{cell::RefCell, collections::HashMap, net::Ipv4Addr, rc::Rc};
use shared::{BackEndRequest, drone::{Descriptor, Request, Update}};
use web_sys::HtmlInputElement;
use yew::{prelude::*, web_sys::HtmlTextAreaElement};

enum Xbee {
    Connected {
        addr: Ipv4Addr,
        signal: Result<i32, String>,
        battery: Result<i32, String>,
        terminal: String,
    },
    Disconnected,
}

enum UpCore {
    Connected {
        addr: Ipv4Addr,
        signal: Result<i32, String>,
        terminal: String,
    },
    Disconnected,
}

pub struct Instance {
    pub descriptor: Descriptor,
    pub optitrack_pos: [f32; 3],
    upcore: UpCore,
    upcore_power: bool,
    xbee: Xbee,
    pixhawk_power: bool,
    camera_stream: HashMap<String, Result<String, String>>,
}

// a lot of stuff here seems like it should be implemented directly on the component,
// update for instance would be cleaner if it was implemented via a ComponentLink<T>
impl Instance {
    pub fn new(descriptor: Descriptor) -> Self {
        Self { 
            descriptor, 
            optitrack_pos: [0.0, 0.0, 0.0],
            upcore: UpCore::Disconnected,
            upcore_power: false,
            xbee: Xbee::Disconnected,
            pixhawk_power: false,
            camera_stream: Default::default(),
        }
    }

    pub fn update(&mut self, update: Update) {
        match update {
            Update::Battery(reading) => if let Xbee::Connected { battery, ..} = &mut self.xbee {
                *battery = Ok(reading);
            },
            Update::Camera { camera, result } => {
                self.camera_stream
                    .insert(camera, result
                        .map(|bytes| base64::encode(bytes)));
            },
            Update::FernbedienungConnected(addr) => 
                self.upcore = UpCore::Connected {
                    addr,
                    signal: Err(String::from("Unknown")),
                    terminal: Default::default(),
                },
            Update::FernbedienungDisconnected => 
                self.upcore = UpCore::Disconnected,
            Update::FernbedienungSignal(strength) => 
                if let UpCore::Connected { signal, ..} = &mut self.upcore {
                    *signal = Ok(strength);
                },
            Update::XbeeConnected(addr) => 
                self.xbee = Xbee::Connected {
                    addr,
                    battery: Err(String::from("Unknown")),
                    signal: Err(String::from("Unknown")),
                    terminal: Default::default(),
                },
            Update::XbeeDisconnected => 
                self.xbee = Xbee::Disconnected,
            Update::XbeeSignal(strength) => if let Xbee::Connected { signal, ..} = &mut self.xbee {
                    *signal = Ok(strength);
            },
            Update::Bash(response) => if let UpCore::Connected { terminal, ..} = &mut self.upcore {
                terminal.push_str(&response);
            },
            Update::Mavlink(response) => if let Xbee::Connected { terminal, ..} = &mut self.xbee {
                terminal.push_str(&response);
            },
            Update::PowerState { upcore, pixhawk } => {
                self.pixhawk_power = pixhawk;
                self.upcore_power = upcore;
            },
        }
    }
}

pub struct Card {
    link: ComponentLink<Self>,
    props: Props,
    // where multiple noderefs etc are present, perhaps
    // this indicates when a component is necessary?
    bash_terminal_visible: bool,
    bash_textarea: NodeRef,
    bash_input: NodeRef,
    // mavlink vs. bash also indicates that a component
    // would be useful
    mavlink_terminal_visible: bool,
    mavlink_textarea: NodeRef,
    mavlink_input: NodeRef,
    camera_dialog_active: bool,
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub instance: Rc<RefCell<Instance>>,
    pub parent: ComponentLink<crate::UserInterface>,
}

pub enum Msg {
    ToggleBashTerminal,
    ToggleMavlinkTerminal,
    ToggleCameraStream,
    SendBashCommand,
    SendMavlinkCommand,
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
            mavlink_terminal_visible: false,
            mavlink_textarea: NodeRef::default(),
            mavlink_input: NodeRef::default(),
            camera_dialog_active: false
        }
    }


    fn rendered(&mut self, _: bool) {
        if let Some(textarea) = self.bash_textarea.cast::<HtmlTextAreaElement>() {
            textarea.set_scroll_top(textarea.scroll_height());
        }
        if let Some(textarea) = self.mavlink_textarea.cast::<HtmlTextAreaElement>() {
            textarea.set_scroll_top(textarea.scroll_height());
        }
    }


    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        let mut drone = self.props.instance.borrow_mut();
        match msg {
            Msg::SendMavlinkCommand => match self.mavlink_input.cast::<HtmlInputElement>() {
                Some(input) => {
                    let drone_request = Request::MavlinkTerminalRun(input.value());
                    input.set_value("");
                    let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                    self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                    true
                },
                _ => false
            },
            Msg::SendBashCommand => match self.bash_input.cast::<HtmlInputElement>() {
                Some(input) => {
                    let drone_request = Request::BashTerminalRun(input.value());
                    input.set_value("");
                    let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                    self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                    true
                },
                _ => false
            },
            Msg::ToggleBashTerminal => {
                match self.bash_terminal_visible {
                    false => {
                        if let UpCore::Connected { terminal, .. } = &mut drone.upcore {
                            terminal.clear();
                        }
                        let drone_request = Request::BashTerminalStart;
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.bash_terminal_visible = true;
                    },
                    true => {
                        let drone_request = Request::BashTerminalStop;
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.bash_terminal_visible = false;
                    }
                }
                true
            },
            Msg::ToggleMavlinkTerminal => {
                match self.mavlink_terminal_visible {
                    false => {
                        if let Xbee::Connected { terminal, .. } = &mut drone.xbee {
                            terminal.clear();
                        }
                        let drone_request = Request::MavlinkTerminalStart;
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.mavlink_terminal_visible = true;
                    },
                    true => {
                        let drone_request = Request::MavlinkTerminalStop;
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        self.mavlink_terminal_visible = false;
                    }
                }
                true
            },
            Msg::ToggleCameraStream => {
                match self.camera_dialog_active {
                    false => {
                        let drone_request = Request::CameraStreamEnable(true);
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
                        self.props.parent.send_message(crate::Msg::SendRequest(request, None));
                        drone.camera_stream.clear();
                        self.camera_dialog_active = true;
                    },
                    true => {
                        let drone_request = Request::CameraStreamEnable(false);
                        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
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
        let drone = self.props.instance.borrow();
        let (batt_level, batt_info) = match &drone.xbee {
            Xbee::Disconnected => (0, String::from("Unknown")),
            Xbee::Connected { battery, .. } => match battery {
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
                            <p class="level-item subtitle is-size-4">{ &drone.descriptor.id }</p>
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
                        { self.render_upcore(&drone) }
                        { self.render_xbee(&drone) }
                        { self.render_identifiers(&drone) }
                    </div>
                </div>
                { self.render_menu(&drone) }
                { self.render_camera_modal(&drone) }
            </div>
        }
    }
}

impl Card {
    fn render_camera_modal(&self, drone: &Instance) -> Html {
        if self.camera_dialog_active {
            let disable_onclick = self.link.callback(|_| Msg::ToggleCameraStream);
            html! {
                <div class="modal is-active">
                    <div class="modal-background" onclick=disable_onclick />
                    <div style="width:50%" class="modal-content">
                        <div class="container is-clipped">
                            <div class="columns is-multiline is-mobile"> { 
                                drone.camera_stream.iter().map(|(id, result)| match result {
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

    fn render_upcore(&self, drone: &Instance) -> Html {
        let (wifi_signal_level, wifi_signal_info) = match &drone.upcore {
            UpCore::Disconnected => (0, String::from("Disconnected")),
            UpCore::Connected { signal, .. } => match signal {
                Err(message) => (0, message.clone()),
                Ok(level) => (match level + 90 {
                    0..=24 => 1,
                    25..=49 => 2,
                    50..=74 => 3,
                    _ => 4,
                }, format!("{}%", level + 90))
            }
        };
        let (term_disabled, term_content) = match &drone.upcore {
            UpCore::Disconnected => (true, String::new()),
            UpCore::Connected { terminal, ..} => (false, terminal.clone())
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
                        <p class="level-item">{ "Up Core" }</p>
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
                                drone.descriptor.upcore_macaddr.to_string()
                            } </p>
                        </div>
                    </div>
                    <div class="column is-two-fifths">
                        <div class="notification has-text-centered">
                            <p style="line-height:32px"> {
                                match drone.upcore {
                                    UpCore::Connected { addr, .. } => addr.to_string(),
                                    UpCore::Disconnected => "Disconnected".to_owned()
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
    
    fn render_xbee(&self, drone: &Instance) -> Html {
        let (wifi_signal_level, wifi_signal_info) = match &drone.xbee {
            Xbee::Disconnected => (0, String::from("Disconnected")),
            Xbee::Connected { signal, .. } => match signal {
                Err(message) => (0, message.clone()),
                Ok(level) => (match level {
                    0..=24 => 1,
                    25..=49 => 2,
                    50..=74 => 3,
                    _ => 4,
                }, format!("{}%", level))
            }
        };
        let (term_disabled, term_content) = match &drone.xbee {
            Xbee::Disconnected => (true, String::new()),
            Xbee::Connected { terminal, ..} => (false, terminal.clone())
        };
        let mut term_classes = classes!("column", "is-full");
        if !self.mavlink_terminal_visible {
            term_classes.push("is-hidden");
        }
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleMavlinkTerminal);
        let term_onkeydown = self.link.batch_callback(|event: KeyboardEvent| match event.key().as_ref() {
            "Enter" => Some(Msg::SendMavlinkCommand),
            _ => None,
        });
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Xbee" }</p>
                    </div>
                    <div class="level-right">
                        <button class="level-item button" onclick=term_btn_onclick disabled=term_disabled> {
                            if self.mavlink_terminal_visible {
                                "Close Mavlink terminal"
                            }
                            else {
                                "Open Mavlink terminal"
                            }
                        } </button>
                    </div>
                </nav>
                
                <div class="columns is-multiline is-mobile">
                    <div class=term_classes>
                        <div>
                            <div class="field">
                                <div class="control">
                                    <textarea ref=self.mavlink_textarea.clone()
                                            class="textarea is-family-monospace"
                                            readonly=false>
                                            { term_content }
                                    </textarea>
                                </div>
                            </div>
                            <div class="field">
                                <div class="control">
                                    <input ref=self.mavlink_input.clone()
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
                                drone.descriptor.xbee_macaddr.to_string()
                            } </p>
                        </div>
                    </div>
                    <div class="column is-two-fifths">
                        <div class="notification has-text-centered">
                            <p style="line-height:32px"> {
                                match drone.xbee {
                                    Xbee::Connected { addr, .. } => addr.to_string(),
                                    Xbee::Disconnected => "Disconnected".to_owned()
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

    fn render_identifiers(&self, drone: &Instance) -> Html {
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
                                drone.descriptor.optitrack_id
                                    .map_or_else(|| "-".to_owned(), |id| id.to_string())
                            } </p>
                        </div>
                    </div>
                    <div class="column is-four-fifths">
                        <div class="notification">
                            <nav class="level is-mobile"> {
                                drone.optitrack_pos.iter().map(|coord| html! {
                                    <p style="line-height:32px" class="level-item">{ format!("{:.3}", coord) }</p>
                                }).collect::<Html>()  
                            } </nav>
                        </div>
                    </div>
                </div>
            </>
        }
    }

    fn render_menu(&self, drone: &Instance) -> Html {
        let toggle_camera_stream_onclick = self.link.callback(|_| Msg::ToggleCameraStream);

        let drone_request = Request::PixhawkPowerEnable(true);
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let power_on_pixhawk_onclick = 
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));
        
        let drone_request = Request::PixhawkPowerEnable(false);
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let power_off_pixhawk_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let drone_request = Request::UpCorePowerEnable(true);
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let power_on_upcore_onclick = 
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));
        
        let drone_request = Request::UpCorePowerEnable(false);
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let power_off_upcore_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let drone_request = Request::UpCoreReboot;
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let reboot_upcore_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let drone_request = Request::UpCoreHalt;
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let halt_upcore_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        let drone_request = Request::Identify;
        let request = BackEndRequest::DroneRequest(drone.descriptor.id.clone(), drone_request);
        let identify_onclick =
            self.props.parent.callback(move |_| crate::Msg::SendRequest(request.clone(), None));

        html! {
            <footer class="card-footer">
                {
                    match drone.upcore {
                        UpCore::Connected {..} => html! {
                            <>
                                <a class="card-footer-item" onclick=toggle_camera_stream_onclick>{ "Show cameras" }</a>
                                <a class="card-footer-item" onclick=identify_onclick>{ "Identify" }</a>
                            </>
                        },
                        UpCore::Disconnected => html! {
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
                            <span>{ "Up Core" }</span>
                            <span class="icon is-small">
                                <i class="mdi mdi-menu-down" />
                            </span>
                        </a>
                    </div>
                    <div class="dropdown-menu" id="dropdown-menu" role="menu">
                        <div class="dropdown-content"> {
                            match drone.upcore {
                                UpCore::Connected {..} => html! {
                                    <a class="dropdown-item" onclick=halt_upcore_onclick>{ "Halt" }</a>
                                },
                                UpCore::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Halt" }</p>
                                },
                            }
                        } {
                            match drone.upcore {
                                UpCore::Connected {..} => html! {
                                    <a class="dropdown-item" onclick=reboot_upcore_onclick>{ "Reboot" }</a>
                                },
                                UpCore::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Reboot" }</p>
                                },
                            }
                        } {
                            match drone.xbee {
                                Xbee::Connected { .. } => match drone.upcore_power {
                                    true => html! {
                                        <a class="dropdown-item" onclick=power_off_upcore_onclick>{ "Power Off" }</a>
                                    },
                                    false => html! {
                                        <a class="dropdown-item" onclick=power_on_upcore_onclick>{ "Power On" }</a>
                                    }
                                }
                                Xbee::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Power On" }</p>
                                }
                            }
                        } </div>
                    </div>
                </div>
                <div class="card-footer-item dropdown is-hoverable">
                    <div class="dropdown-trigger">
                        <a>
                            <span>{ "Pixhawk" }</span>
                            <span class="icon is-small">
                                <i class="mdi mdi-menu-down" />
                            </span>
                        </a>
                    </div>
                    <div class="dropdown-menu" id="dropdown-menu" role="menu">
                        <div class="dropdown-content"> {
                            match drone.xbee {
                                Xbee::Connected { .. } => match drone.pixhawk_power {
                                    true => html! {
                                        <a class="dropdown-item" onclick=power_off_pixhawk_onclick>{ "Power Off" }</a>
                                    },
                                    false => html! {
                                        <a class="dropdown-item" onclick=power_on_pixhawk_onclick>{ "Power On" }</a>
                                    }
                                }
                                Xbee::Disconnected => html! {
                                    <p class="dropdown-item has-text-grey-light">{ "Power On" }</p>
                                }
                            }
                        } </div>
                    </div>
                </div>
            </footer>
        }
    }
}
