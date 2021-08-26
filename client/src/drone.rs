use std::{cell::RefCell, net::Ipv4Addr, rc::Rc};

use shared::drone;
use yew::{prelude::*, services::ConsoleService};
//use wasm_bindgen::prelude::*;

enum Terminal {
    Active {
        buffer: Vec<String>
    },
    Inactive,
}

enum Pixhawk {
    Connected {
        ip: Ipv4Addr,
        signal: u8,
        terminal: Terminal,
    },
    Disconnected,
}

enum UpCore {
    Connected {
        ip: Ipv4Addr,
        signal: u8,
        terminal: Terminal,
    },
    Disconnected,
}


pub struct Instance {
    descriptor: drone::Descriptor,
    battery: u8,
    optitrack_pos: [f32; 3],
    upcore: UpCore,
    pixhawk: Pixhawk,
}

impl Instance {
    pub fn new(descriptor: drone::Descriptor) -> Self {
        Self { 
            descriptor, 
            battery: 10, 
            optitrack_pos: [0.0, 0.0, 0.0],
            upcore: UpCore::Disconnected,
            pixhawk: Pixhawk::Disconnected
        }
    }

    pub fn update(&mut self, update: shared::drone::Update) {
    }
}

pub struct Card {
    link: ComponentLink<Self>,
    props: Props,
    bash_terminal_visible: bool,
    mavlink_terminal_visible: bool,
    camera_modal_active: bool
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub instance: Rc<RefCell<Instance>>,
}

pub enum Msg {
    ToggleBashTerminal,
    ToggleMavlinkTerminal,
    ToggleCameraStream,
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
            camera_modal_active: false
        }
    }

    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::ToggleBashTerminal => {
                self.bash_terminal_visible = !self.bash_terminal_visible;
                true
            },
            Msg::ToggleMavlinkTerminal => {
                self.mavlink_terminal_visible = !self.mavlink_terminal_visible;
                true
            },
            Msg::ToggleCameraStream => {
                self.camera_modal_active = !self.camera_modal_active;
                true
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
        html! {
            <div class="card">
                <header class="card-header">
                    <nav class="card-header-title is-shadowless has-background-white-ter level is-mobile">
                        <div class="level-left">
                            <p class="level-item subtitle is-size-4">{ &drone.descriptor.id }</p>
                        </div>
                        <div class="level-right">
                            <figure class="level-item image mx-0 is-48x48">
                                <img src=format!("images/batt{}.svg", match drone.battery {
                                    0..=24 => '1',
                                    25..=49 => '2',
                                    50..=74 => '3',
                                    _ => '4',
                                }) />
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
        if self.camera_modal_active {
            let disable_onclick = self.link.callback(|_| Msg::ToggleCameraStream);
            html! {
                <div class="modal is-active">
                    <div class="modal-background" onclick=disable_onclick />
                    <div class="modal-content">
                        <div class="columns is-multiline is-mobile">
                            <div class="column is-half">
                                <figure class="image is-128x128">
                                    <img src="https://image-placeholder.com/images/actual-size/640x480.png" alt="" />
                                </figure>
                            </div>
                            <div class="column is-half">
                                <figure class="image is-128x128">
                                    <img src="https://image-placeholder.com/images/actual-size/640x480.png" alt="" />
                                </figure>
                            </div>
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
        let wifi_signal_strength = format!("images/wifi{}.svg", match drone.upcore {
            UpCore::Disconnected => '0',
            UpCore::Connected { signal, .. } => match signal {
                0..=24 => '1',
                25..=49 => '2',
                50..=74 => '3',
                _ => '4',
            },
        });
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleBashTerminal);
        let term_disabled = match &drone.upcore {
            UpCore::Connected { terminal, .. } => match terminal {
                Terminal::Active { .. } => false,
                Terminal::Inactive => true,
            },
            UpCore::Disconnected => true,
        };
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
                                        <div class="control">
                                            <textarea class="textarea"
                                                      readonly=true
                                                      disabled=term_disabled />
                                        </div>
                                    </div>
                                    <div class="field">
                                        <div class="control">
                                            <input class="input"
                                                   type="text" 
                                                   disabled=term_disabled
                                                   placeholder="Type a command and press enter" />
                                        </div>
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
                                    UpCore::Connected { ip, .. } => ip.to_string(),
                                    UpCore::Disconnected => "Disconnected".to_owned()
                                }
                            } </p>
                        </div>
                    </div>
                    <div class="column is-one-fifth">
                        <div class="notification has-text-centered">
                            <figure class="image mx-auto is-32x32">
                                <img src=wifi_signal_strength />
                            </figure>
                        </div>
                    </div>
                </div>
            </>
        }
    }
    
    fn render_pixhawk(&self, drone: &Instance) -> Html {
        let wifi_signal_strength = format!("images/wifi{}.svg", match drone.pixhawk {
            Pixhawk::Disconnected => '0',
            Pixhawk::Connected { signal, .. } => match signal {
                0..=24 => '1',
                25..=49 => '2',
                50..=74 => '3',
                _ => '4',
            },
        });
        let term_btn_onclick = self.link.callback(|_| Msg::ToggleMavlinkTerminal);
        let term_disabled = match &drone.pixhawk {
            Pixhawk::Connected { terminal, .. } => match terminal {
                Terminal::Active { .. } => false,
                Terminal::Inactive => true,
            },
            Pixhawk::Disconnected => true,
        };
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
                                        <div class="control">
                                            <textarea class="textarea"
                                                      readonly=true
                                                      disabled=term_disabled />
                                        </div>
                                    </div>
                                    <div class="field">
                                        <div class="control">
                                            <input class="input"
                                                   type="text" 
                                                   disabled=term_disabled
                                                   placeholder="Type a command and press enter" />
                                        </div>
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
                                    Pixhawk::Connected { ip, .. } => ip.to_string(),
                                    Pixhawk::Disconnected => "Disconnected".to_owned()
                                }
                            } </p>
                        </div>
                    </div>
                    <div class="column is-one-fifth">
                        <div class="notification has-text-centered">
                            <figure class="image mx-auto is-32x32">
                                <img src=wifi_signal_strength />
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
                            <a class="dropdown-item">{ "Halt" }</a>
                            <a class="dropdown-item">{ "Reboot" }</a>
                            <a class="dropdown-item">{ "Power Off" }</a>
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

// <nav class="level is-mobile">
//                 <div class="level-item">
//                     <div class="dropdown is-hoverable">
//                         <div class="dropdown-trigger">
//                             <button class="button">
//                                 <span>{ "Up Core" }</span>
//                                 <span class="icon is-small">
//                                     <i class="mdi mdi-menu-down" />
//                                 </span>
//                             </button>
//                         </div>
//                         <div class="dropdown-menu" id="dropdown-menu" role="menu">
//                             <div class="dropdown-content">
//                                 <a class="dropdown-item">{ "Halt" }</a>
//                                 <a class="dropdown-item">{ "Reboot" }</a>
//                                 <a class="dropdown-item">{ "Power Off" }</a>
//                             </div>
//                         </div>
//                     </div>
//                 </div>
//                 <div class="level-item">
//                     <div class="dropdown is-hoverable">
//                         <div class="dropdown-trigger">
//                             <button class="button">
//                                 <span>{ "Pixhawk" }</span>
//                                 <span class="icon is-small">
//                                     <i class="mdi mdi-menu-down" />
//                                 </span>
//                             </button>
//                         </div>
//                         <div class="dropdown-menu" id="dropdown-menu" role="menu">
//                             <div class="dropdown-content">
//                                 <a class="dropdown-item">{ "Power Off" }</a>
//                             </div>
//                         </div>
//                     </div>
//                 </div>
//                 <div class="level-item">
//                     <button class="button">{ "Show cameras" }</button>
//                 </div>
//             </nav>