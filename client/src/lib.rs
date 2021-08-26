use std::{cell::RefCell, convert::AsRef, rc::Rc};
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{AsRefStr, EnumIter, EnumProperty};
use wasm_bindgen::prelude::*;
use yew::prelude::*;
use yew::services::websocket::{WebSocketService, WebSocketStatus, WebSocketTask};
use yew::services::ConsoleService;

mod drone;

#[derive(AsRefStr, EnumProperty, EnumIter, Copy, Clone, PartialEq)]
enum Tab {
    #[strum(serialize = "Drones", props(icon = "mdi-quadcopter"))]
    Drones,
    #[strum(serialize = "Pi-Pucks", props(icon = "mdi-circle-slice-8"))]
    PiPucks,
    #[strum(serialize = "Experiment", props(icon = "mdi-play"))]
    Experiment,
}

struct UserInterface {
    link: ComponentLink<Self>,
    socket: Option<WebSocketTask>,
    active_tab: Tab,
    drones: Vec<Rc<RefCell<drone::Instance>>>,
}



enum Msg {

    // TODO: handle disconnects by matching against WebSocketStatus::Closed or WebSocketStatus::Error
    Notifcation(WebSocketStatus),
    Data(Result<Vec<u8>, anyhow::Error>),
    SetActiveTab(Tab),
}

impl Component for UserInterface {
    type Message = Msg;
    type Properties = ();

    fn create(_props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let service_addr = yew::utils::document()
            .location()
            .unwrap()
            .host()
            .unwrap();
        let service_addr = format!("ws://{}/socket", service_addr);
        let callback_data =
            link.callback(|data| Msg::Data(data));
        let callback_notification =
            link.callback(|notification| Msg::Notifcation(notification));
        let socket =
            WebSocketService::connect_binary(&service_addr,
                                             callback_data,
                                             callback_notification);
        // Delete me
        let desc = shared::drone::Descriptor {
            id: "drone1".into(),
            xbee_macaddr: [0,0,0,0,0,0].into(),
            upcore_macaddr: [0,0,0,0,0,0].into(),
            optitrack_id: Some(42),
        };
        let desc_vec = vec![Rc::new(RefCell::new(drone::Instance::new(desc)))];
        // Delete me
        Self {
            link,
            socket: match socket {
                Ok(socket) => Some(socket),
                Err(_) => {
                    ConsoleService::log("Could not connect to socket");
                    None
                }
            },
            active_tab: Tab::Drones,
            drones: desc_vec, // Default::default(),
        }
    }

    // pub enum DownMessage {
    //     // broadcast, trigger by change in actual drone
    //     UpdateDrone(String, drone::Update),
    //     UpdatePiPuck(String, pipuck::Update),
    // }
    

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::SetActiveTab(tab) => {
                self.active_tab = tab;
                true
            }
            Msg::Data(data) => match data {
                Ok(data) => match bincode::deserialize::<shared::DownMessage>(&data) {
                    Ok(decoded) => match decoded {
                        shared::DownMessage::AddDrone(descriptor) => {
                            // if self.drones.iter().find(|drone| drone.descriptor.id == descriptor.id).is_none() {
                            //     self.drones.push(drone::Instance::new(descriptor));
                            // }
                            true
                        }
                        shared::DownMessage::UpdateDrone(id, update) => {
                            // match self.drones.iter_mut().find(|drone| drone.descriptor.id == id) {
                            //     Some(drone) => drone.update(update),
                            //     None => ConsoleService::log(&format!("Drone {} not added to interface", id)),
                            // }
                            true
                        },
                        shared::DownMessage::AddPiPuck(descriptor) => {
                            // if self.pipucks.iter().find(|pipuck| pipuck.descriptor.id == descriptor.id).is_none() {
                            //     self.pipucks.push(pipuck::PiPuck::new(descriptor));
                            // }
                            true
                        }
                        shared::DownMessage::UpdatePiPuck(id, update) => {
                            true
                        },
                    },
                    Err(error) => {
                        ConsoleService::log(&format!("1. {:?}", error));
                        false
                    }
                },
                Err(err) => {
                    ConsoleService::log(&format!("2. {:?}", err));
                    false
                },
            },
            Msg::Notifcation(notification) => {
                ConsoleService::log(&format!("{:?}", notification));
                true
            }
        }
    }


    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            <>
                { self.render_hero() }
                { self.render_tabs() }
                <section class="section">
                    <div class="container is-fluid">
                        <div class="columns is-multiline is-mobile">
                            <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd"> {
                                match self.active_tab {
                                    Tab::Drones => {
                                        self.drones.iter().map(|drone| html!{
                                            <drone::Card instance=drone />
                                        }).collect::<Html>()
                                    }
                                    Tab::PiPucks => {
                                        html! {}
                                    }
                                    Tab::Experiment => {
                                        html! {}
                                    }
                                }
                            }
                            </div>
                        </div>
                    </div>
                </section>
            </>
        }
    }
}

impl UserInterface {
    fn render_hero(&self) -> Html {
        html!{
            <section class="hero is-link">
                <div class="hero-body">
                    <div class="columns is-vcentered">
                        <div class="column is-narrow">
                            <figure class="image is-64x64">
                                <img src="images/drone.png" />
                            </figure>
                        </div>
                        <div class="column">
                            <p class="title is-2">{ "Supervisor" }</p>
                        </div>
                    </div>
                </div>
            </section>
        }
    }

    fn render_tabs(&self) -> Html {
        html! {
            <div class="tabs is-centered is-boxed is-medium">
                <ul> {
                    Tab::iter()
                        .map(|tab| {
                            let li_classes = if self.active_tab == tab {
                                Some("is-active")
                            }
                            else {
                                None
                            };
                            let i_classes = ["mdi", "mdi-24px", tab.get_str("icon").unwrap()];
                            let tab_name = tab.as_ref();
                            let onclick = self.link.callback(move |_| Msg::SetActiveTab(tab));
                            html! {
                                <li class=classes!(li_classes)>
                                    <a onclick=onclick>
                                        <span class="icon is-medium">
                                            <i class=classes!(&i_classes[..])></i>
                                        </span>
                                        <span>{ tab_name }</span>
                                    </a>
                                </li>
                            }
                        })
                        .collect::<Html>()
                } </ul>
            </div>
        }
    }
}


#[wasm_bindgen]
pub fn launch() -> Result<(), JsValue> {
    yew::start_app::<UserInterface>();
    Ok(())
}
