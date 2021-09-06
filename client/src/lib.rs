use std::{cell::RefCell, collections::HashMap, convert::AsRef, rc::Rc};
use anyhow::Context;
use shared::DownMessage;
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{AsRefStr, EnumIter, EnumProperty};
use wasm_bindgen::prelude::*;
use yew::prelude::*;
use yew::services::websocket::{WebSocketService, WebSocketStatus, WebSocketTask};
use yew::services::ConsoleService;

mod drone;
mod experiment;

#[derive(AsRefStr, EnumProperty, EnumIter, Copy, Clone, PartialEq)]
pub enum Tab {
    #[strum(serialize = "Drones", props(icon = "mdi-quadcopter"))]
    Drones,
    #[strum(serialize = "Pi-Pucks", props(icon = "mdi-circle-slice-8"))]
    PiPucks,
    #[strum(serialize = "Experiment", props(icon = "mdi-play"))]
    Experiment,
}

pub struct UserInterface {
    link: ComponentLink<Self>,
    socket: Option<WebSocketTask>,
    active_tab: Tab,
    drones: HashMap<String, Rc<RefCell<drone::Instance>>>,
    //pipucks: HashMap<shared::pipuck::Descriptor, Rc<RefCell<pipuck::Instance>>>,
    drone_config_comp: Option<ComponentLink<experiment::DroneConfigCard>>,
}



pub enum Msg {

    // TODO: handle disconnects by matching against WebSocketStatus::Closed or WebSocketStatus::Error
    WebSocketNotifcation(WebSocketStatus),
    WebSocketRxData(Result<Vec<u8>, anyhow::Error>),
    SendUpMessage(shared::UpMessage),
    SetActiveTab(Tab),

    RegisterDroneConfigComp(ComponentLink<experiment::DroneConfigCard>),
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
            link.callback(|data| Msg::WebSocketRxData(data));
        let callback_notification =
            link.callback(|notification| Msg::WebSocketNotifcation(notification));
        let socket =
            WebSocketService::connect_binary(&service_addr,
                                             callback_data,
                                             callback_notification);
        Self {
            link,
            socket: match socket {
                Ok(socket) => Some(socket),
                Err(_) => {
                    ConsoleService::log("Could not connect to socket");
                    None
                }
            },
            active_tab: Tab::Experiment,
            drones: Default::default(),
            drone_config_comp: None,
            //pipucks: Default::default(),
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::SetActiveTab(tab) => {
                self.active_tab = tab;
                true
            }
            Msg::SendUpMessage(message) => {
                if let Some(websocket) = &mut self.socket {
                    websocket.send_binary(bincode::serialize(&message).context("Could not serialize UpMessage"));  
                }
                else {
                    ConsoleService::log("Could not send UpMessage: Not connected");
                }
                false
            }
            Msg::WebSocketRxData(data) => match data {
                Ok(data) => match bincode::deserialize::<DownMessage>(&data) {
                    Ok(decoded) => match decoded {
                        DownMessage::AddDrone(desc) => {
                            self.drones.entry(desc.id.clone())
                                .or_insert_with(|| Rc::new(RefCell::new(drone::Instance::new(desc))));
                            true
                        },
                        DownMessage::UpdateDrone(id, update) => {
                            if let Some(drone) = self.drones.get(&id) {
                                drone.borrow_mut().update(update);
                            }
                            true
                        },
                        DownMessage::UpdateExperiment(update) => {
                            ConsoleService::log("got update exp");
                            match update {
                                shared::experiment::Update::State(x) => {},
                                shared::experiment::Update::DroneSoftware { checksums, status } => {
                                    ConsoleService::log("got drone update soft");
                                    if let Some(drone_config_comp) = &self.drone_config_comp {
                                        let comp_msg = experiment::Msg::UpdateSoftware(checksums, status);
                                        drone_config_comp.send_message(comp_msg);
                                    }
                                },
                                shared::experiment::Update::PiPuckSoftware { checksums, status } => {

                                }
                            }
                            true
                        }
                        _ => {
                            // TODO
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
            Msg::WebSocketNotifcation(notification) => {
                ConsoleService::log(&format!("{:?}", notification));
                true
            }
            Msg::RegisterDroneConfigComp(link) => {
                ConsoleService::log("registered drone comp link");
                self.drone_config_comp = Some(link);
                false
            },
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
                                    Tab::Drones => self.drones
                                        .values()
                                        .map(|drone| html!{
                                            <drone::Card instance=drone parent=self.link.clone() />
                                        }).collect::<Html>(),
                                    Tab::PiPucks => {
                                        html! {}
                                    },
                                    Tab::Experiment => html! {
                                        <experiment::DroneConfigCard parent=self.link.clone() />
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
