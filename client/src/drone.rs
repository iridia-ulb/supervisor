use std::{cell::RefCell, collections::HashMap, net::Ipv4Addr, rc::Rc};
use shared::{UpMessage, drone::{Action, Descriptor, FernbedienungAction, BashAction, XbeeAction, MavlinkAction, Update}};
use yew::{prelude::*, services::ConsoleService};
//use wasm_bindgen::prelude::*;

enum Pixhawk {
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
    optitrack_pos: [f32; 3],
    upcore: UpCore,
    upcore_power: bool,
    pixhawk: Pixhawk,
    pixhawk_power: bool,
    camera_stream: HashMap<String, Result<String, String>>,
}

impl Instance {
    pub fn new(descriptor: Descriptor) -> Self {
        Self { 
            descriptor, 
            optitrack_pos: [0.0, 0.0, 0.0],
            upcore: UpCore::Disconnected,
            upcore_power: false,
            pixhawk: Pixhawk::Disconnected,
            pixhawk_power: false,
            camera_stream: Default::default(),
        }
    }

    pub fn update(&mut self, update: Update) {
        match update {
            Update::Battery(reading) => if let Pixhawk::Connected { battery, ..} = &mut self.pixhawk {
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
                self.pixhawk = Pixhawk::Connected {
                    addr,
                    battery: Err(String::from("Unknown")),
                    signal: Err(String::from("Unknown")),
                    terminal: Default::default(),
                },
            Update::XbeeDisconnected => 
                self.pixhawk = Pixhawk::Disconnected,
            Update::XbeeSignal(strength) => if let Pixhawk::Connected { signal, ..} = &mut self.pixhawk {
                    *signal = Ok(strength);
            },
            Update::Bash(response) => if let UpCore::Connected { terminal, ..} = &mut self.upcore {
                terminal.push_str(&response);
            },
            Update::Mavlink(response) => if let Pixhawk::Connected { terminal, ..} = &mut self.pixhawk {
                terminal.push_str(&response);
            },
            Update::PowerState { upcore, pixhawk } => {
                self.pixhawk_power = pixhawk;
                self.upcore_power = upcore;
            }
        }
    }
}

pub struct Card {
    link: ComponentLink<Self>,
    props: Props,
    bash_terminal_visible: bool,
    mavlink_terminal_visible: bool,
    camera_dialog_active: bool
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
    SendAction(Action),
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
            mavlink_terminal_visible: false,
            camera_dialog_active: false
        }
    }

    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        let mut drone = self.props.instance.borrow_mut();
        match msg {
            Msg::ToggleBashTerminal => {
                match self.bash_terminal_visible {
                    false => {
                        if let UpCore::Connected { terminal, .. } = &mut drone.upcore {
                            terminal.clear();
                        }
                        self.bash_terminal_visible = true;
                    },
                    true => {
                        let action = Action::Fernbedienung(FernbedienungAction::Bash(BashAction::Close));
                        let message = UpMessage::DroneAction(drone.descriptor.id.clone(), action);
                        self.props.parent.send_message(crate::Msg::SendUpMessage(message));
                        self.bash_terminal_visible = false;
                    }
                }
                true
            },
            Msg::ToggleMavlinkTerminal => {
                self.mavlink_terminal_visible = !self.mavlink_terminal_visible;
                true
            },
            Msg::ToggleCameraStream => {
                match self.camera_dialog_active {
                    false => {
                        let action = Action::Fernbedienung(FernbedienungAction::SetCameraStream(true));
                        let message = UpMessage::DroneAction(drone.descriptor.id.clone(), action);
                        self.props.parent.send_message(crate::Msg::SendUpMessage(message));
                        drone.camera_stream.clear();
                        self.camera_dialog_active = true;
                    },
                    true => {
                        let action = Action::Fernbedienung(FernbedienungAction::SetCameraStream(false));
                        let message = UpMessage::DroneAction(drone.descriptor.id.clone(), action);
                        self.props.parent.send_message(crate::Msg::SendUpMessage(message));
                        self.camera_dialog_active = false;
                    }
                }
                true
            },
            Msg::SendAction(action) => {
                let message = UpMessage::DroneAction(drone.descriptor.id.clone(), action);
                self.props.parent.send_message(crate::Msg::SendUpMessage(message));
                false
            }
        }
    }

    // this fires when the parent changes the properties of this component
    fn change(&mut self, props: Self::Properties) -> ShouldRender {
        true
    }

    // `self.link.callback(...)` can only be created with a struct that impl Component
    // `|_: ClickEvent| { Msg::Click }` can probably be stored anywhere, i.e., external to the component
    // 
    fn view(&self) -> Html {
        //let toggle_upcore_power = self.link.callback(|e: MouseEvent| Msg::ToggleUpcorePower);
        let drone = self.props.instance.borrow();

        let (batt_level, batt_info) = match &drone.pixhawk {
            Pixhawk::Disconnected => (0, String::from("Unknown")),
            Pixhawk::Connected { battery, .. } => match battery {
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
                        { self.render_pixhawk(&drone) }
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
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleBashTerminal);
        let term_onkeydown = self.link.callback(|e: KeyboardEvent| {
            Msg::SendAction(
                Action::Fernbedienung(
                    FernbedienungAction::Bash(
                        BashAction::KeyDown(e.key()))))
        });
        let term_onkeyup = self.link.callback(|e: KeyboardEvent| {
            Msg::SendAction(
                Action::Fernbedienung(
                    FernbedienungAction::Bash(
                        BashAction::KeyUp(e.key()))))
        });
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Up Core" }</p>
                    </div>
                    <div class="level-right">
                        <button class="level-item button" onclick=term_btn_onclick> {
                            if self.bash_terminal_visible {
                                "Hide Bash terminal"
                            }
                            else {
                                "Show Bash terminal"
                            }
                        } </button>
                    </div>
                </nav>
                
                <div class="columns is-multiline is-mobile"> {
                    match self.bash_terminal_visible {
                        false => html! {},
                        true  => html! {
                            <div class="column is-full">
                                <div class="is-family-monospace">
                                    <div class="field">
                                        <div class="control"> {
                                            match &drone.upcore {
                                                UpCore::Connected { terminal, .. } => html! {
                                                    <textarea class="textarea"
                                                              readonly=true
                                                              onkeydown=term_onkeydown
                                                              onkeyup=term_onkeyup> {
                                                        terminal.as_str()
                                                    } </textarea>
                                                },
                                                UpCore::Disconnected => html! {
                                                    <textarea class="textarea" readonly=true disabled=true />
                                                },
                                            }
                                        } </div>
                                    </div>
                                </div>
                            </div>
                        }
                    }}
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
    
    fn render_pixhawk(&self, drone: &Instance) -> Html {
        let (wifi_signal_level, wifi_signal_info) = match &drone.pixhawk {
            Pixhawk::Disconnected => (0, String::from("Disconnected")),
            Pixhawk::Connected { signal, .. } => match signal {
                Err(message) => (0, message.clone()),
                Ok(level) => (match level {
                    0..=24 => 1,
                    25..=49 => 2,
                    50..=74 => 3,
                    _ => 4,
                }, format!("{}%", level))
            }
        };
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleMavlinkTerminal);
        let term_onkeydown = self.link.callback(|e: KeyboardEvent| {
            Msg::SendAction(
                Action::Xbee(
                    XbeeAction::Mavlink(
                        MavlinkAction::KeyDown(e.key()))))
        });
        let term_onkeyup = self.link.callback(|e: KeyboardEvent| {
            Msg::SendAction(
                Action::Xbee(
                    XbeeAction::Mavlink(
                        MavlinkAction::KeyUp(e.key()))))
        });
        html! {
            <>
                <nav class="level is-mobile">
                    <div class="level-left">
                        <p class="level-item">{ "Pixhawk" }</p>
                    </div>
                    <div class="level-right">
                        <button class="level-item button" onclick=term_btn_onclick> {
                            if self.mavlink_terminal_visible {
                                "Hide Mavlink terminal"
                            }
                            else {
                                "Show Mavlink terminal"
                            }
                        } </button>
                    </div>
                </nav>
                
                <div class="columns is-multiline is-mobile"> {
                    match self.mavlink_terminal_visible {
                        false => html! {},
                        true  => html! {
                            <div class="column is-full">
                                <div class="is-family-monospace">
                                    <div class="field">
                                        <div class="control"> {
                                            match &drone.pixhawk {
                                                Pixhawk::Connected { terminal, .. } => html! {
                                                    <textarea class="textarea"
                                                            readonly=true
                                                            onkeydown=term_onkeydown
                                                            onkeyup=term_onkeyup> {
                                                        terminal.as_str()
                                                    } </textarea>
                                                },
                                                Pixhawk::Disconnected => html! {
                                                    <textarea class="textarea" readonly=true disabled=true />
                                                },
                                            }
                                        } </div>
                                    </div>
                                </div>
                            </div>
                        }
                    }}
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
                                match drone.pixhawk {
                                    Pixhawk::Connected { addr, .. } => addr.to_string(),
                                    Pixhawk::Disconnected => "Disconnected".to_owned()
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
        let toggle_onclick = self.link.callback(|_| Msg::ToggleCameraStream);
        let power_on_upcore_onclick = 
            self.link.callback(|_|
                    Msg::SendAction(Action::Xbee(XbeeAction::SetUpCorePower(true))));
        let power_off_upcore_onclick =
            self.link.callback(|_|
                Msg::SendAction(Action::Xbee(XbeeAction::SetUpCorePower(false))));
        let reboot_upcore_onclick = 
            self.link.callback(|_|
                Msg::SendAction(Action::Fernbedienung(FernbedienungAction::Reboot)));
        let halt_upcore_onclick = 
            self.link.callback(|_|
                Msg::SendAction(Action::Fernbedienung(FernbedienungAction::Halt)));
        html! {
            <footer class="card-footer">
                <a class="card-footer-item" onclick=toggle_onclick>{ "Show cameras" }</a>
                <a class="card-footer-item">{ "Identify" }</a>
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
                        <div class="dropdown-content">
                            <a class="dropdown-item" onclick=halt_upcore_onclick>{ "Halt" }</a>
                            <a class="dropdown-item" onclick=reboot_upcore_onclick>{ "Reboot" }</a>
                            {
                                match drone.pixhawk {
                                    Pixhawk::Connected { .. } => match drone.upcore_power {
                                        true => html! {
                                            <a class="dropdown-item" onclick=power_off_upcore_onclick>{ "Power Off" }</a>
                                        },
                                        false => html! {
                                            <a class="dropdown-item" onclick=power_on_upcore_onclick>{ "Power On" }</a>
                                        }
                                    }
                                    Pixhawk::Disconnected => html! {
                                        <p class="dropdown-item has-text-grey-light">{ "Power On" }</p>
                                    }
                                }
                            }
                        </div>
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
                        <div class="dropdown-content">
                            <a class="dropdown-item">{ "Power Off" }</a>
                        </div>
                    </div>
                </div>
            </footer>
        }
    }
}
