use std::{cell::RefCell, collections::HashMap, convert::AsRef, rc::Rc};
use shared::experiment::software::Software;
use shared::{DownMessage, UpMessage};
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{AsRefStr, EnumIter, EnumProperty};
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use yew::prelude::*;
use yew::services::websocket::{WebSocketService, WebSocketStatus, WebSocketTask};
use yew::services::ConsoleService;

mod builderbot;
mod drone;
mod pipuck;
mod experiment;

#[derive(AsRefStr, EnumProperty, EnumIter, Copy, Clone, PartialEq)]
pub enum Tab {
    #[strum(serialize = "BuilderBots", props(icon = "mdi-crop-square"))]
    BuilderBots,
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
    requests: HashMap<Uuid, Callback<Result<(), String>>>,
    builderbots: HashMap<String, Rc<RefCell<builderbot::Instance>>>,
    builderbot_software: Rc<RefCell<Software>>,
    builderbot_config_comp: Option<ComponentLink<experiment::builderbot::ConfigCard>>,
    drones: HashMap<String, Rc<RefCell<drone::Instance>>>,
    drone_software: Rc<RefCell<Software>>,
    drone_config_comp: Option<ComponentLink<experiment::drone::ConfigCard>>,
    pipucks: HashMap<String, Rc<RefCell<pipuck::Instance>>>,
    pipuck_software: Rc<RefCell<Software>>,
    pipuck_config_comp: Option<ComponentLink<experiment::pipuck::ConfigCard>>,
    control_config_comp: Option<ComponentLink<experiment::Interface>>,
}



pub enum Msg {
    // TODO: handle disconnects by matching against WebSocketStatus::Closed or WebSocketStatus::Error
    WebSocketNotifcation(WebSocketStatus),
    WebSocketRxData(Result<Vec<u8>, anyhow::Error>),
    SetActiveTab(Tab),
    SendRequest(shared::BackEndRequest, Option<Callback<Result<(), String>>>),
    SetBuilderBotConfigComp(ComponentLink<experiment::builderbot::ConfigCard>),
    SetDroneConfigComp(ComponentLink<experiment::drone::ConfigCard>),
    SetPiPuckConfigComp(ComponentLink<experiment::pipuck::ConfigCard>),
    SetControlConfigComp(ComponentLink<experiment::Interface>),
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
            active_tab: Tab::Drones,
            requests: Default::default(),
            builderbots: Default::default(),
            drones: Default::default(),
            pipucks: Default::default(),
            /* configuration component links */
            builderbot_config_comp: None,
            drone_config_comp: None,
            pipuck_config_comp: None,
            control_config_comp: None,
            builderbot_software: Default::default(),
            drone_software: Default::default(),
            pipuck_software: Default::default(),
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::SetActiveTab(tab) => {
                self.active_tab = tab;
                true
            }
            Msg::SendRequest(request, callback) => {
                match self.socket.as_mut() {
                    Some(websocket) => {
                        let id = Uuid::new_v4();
                        let message = UpMessage::Request(id, request);
                        match bincode::serialize(&message) {
                            Ok(serialized) => {
                                websocket.send_binary(Ok(serialized));
                                if let Some(callback) = callback {
                                    self.requests.insert(id, callback);
                                }
                            },
                            Err(error) => if let Some(callback) = callback {
                                callback.emit(Err(format!("Could not serialize request: {}", error)));
                            }
                        }
                    }
                    None => if let Some(callback) = callback {
                        callback.emit(Err(String::from("Could not send request: Disconnected")));
                    }
                }
                false
            }
            Msg::WebSocketRxData(data) => match data {
                Ok(data) => match bincode::deserialize::<DownMessage>(&data) {
                    Ok(decoded) => match decoded {
                        DownMessage::Request(_uuid, request) => match request {
                            shared::FrontEndRequest::AddBuilderBot(desc) => {
                                self.builderbots.entry(desc.id.clone())
                                    .or_insert_with(|| Rc::new(RefCell::new(builderbot::Instance::new(desc))));
                                true
                            },
                            shared::FrontEndRequest::UpdateBuilderBot(id, update) => {
                                if let Some(builderbot) = self.builderbots.get(&id) {
                                    builderbot.borrow_mut().update(update);
                                }
                                true
                            },
                            shared::FrontEndRequest::AddDrone(desc) => {
                                self.drones.entry(desc.id.clone())
                                    .or_insert_with(|| Rc::new(RefCell::new(drone::Instance::new(desc))));
                                true
                            },
                            shared::FrontEndRequest::UpdateDrone(id, update) => {
                                if let Some(drone) = self.drones.get(&id) {
                                    drone.borrow_mut().update(update);
                                }
                                true
                            },
                            shared::FrontEndRequest::AddPiPuck(desc) => {
                                self.pipucks.entry(desc.id.clone())
                                    .or_insert_with(|| Rc::new(RefCell::new(pipuck::Instance::new(desc))));
                                true
                            },
                            shared::FrontEndRequest::UpdatePiPuck(id, update) => {
                                if let Some(pipuck) = self.pipucks.get(&id) {
                                    pipuck.borrow_mut().update(update);
                                }
                                true
                            },
                            shared::FrontEndRequest::UpdateExperiment(_) => todo!(),
                            shared::FrontEndRequest::UpdateTrackingSystem(updates) => {
                                for update in updates {
                                    for builderbot in self.builderbots.values() {
                                        let mut builderbot = builderbot.borrow_mut();
                                        if let Some(id) = builderbot.descriptor.optitrack_id {
                                            if update.id == id {
                                                builderbot.optitrack_pos = update.position;
                                            }
                                        }
                                    }
                                    for drone in self.drones.values() {
                                        let mut drone = drone.borrow_mut();
                                        if let Some(id) = drone.descriptor.optitrack_id {
                                            if update.id == id {
                                                drone.optitrack_pos = update.position;
                                            }
                                        }
                                    }
                                    for pipuck in self.pipucks.values() {
                                        let mut pipuck = pipuck.borrow_mut();
                                        if let Some(id) = pipuck.descriptor.optitrack_id {
                                            if update.id == id {
                                                pipuck.optitrack_pos = update.position;
                                            }
                                        }
                                    }
                                }
                                true
                            },
                        },
                        DownMessage::Response(uuid, result) => {
                            if let Some(callback) = self.requests.remove(&uuid) {
                                if let Err(error) = result.as_ref() {
                                    ConsoleService::log(&format!("Error processing request: {}", error));
                                }
                                callback.emit(result);
                            }
                            false
                        }
                    },
                    Err(error) => {
                        ConsoleService::log(&format!("Could not deserialize backend message: {}", error));
                        false
                    }
                },
                Err(error) => {
                    ConsoleService::log(&format!("Could not recieve backend message: {}", error));
                    false
                },
            },
            Msg::WebSocketNotifcation(notification) => {
                ConsoleService::log(&format!("Connection to backend: {:?}", notification));
                false
            }
            Msg::SetBuilderBotConfigComp(link) => {
                self.builderbot_config_comp = Some(link);
                false
            },
            Msg::SetDroneConfigComp(link) => {
                self.drone_config_comp = Some(link);
                false
            },
            Msg::SetPiPuckConfigComp(link) => {
                self.pipuck_config_comp = Some(link);
                false
            },
            Msg::SetControlConfigComp(link) => {
                self.control_config_comp = Some(link);
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
                        <div class="columns is-multiline is-mobile"> {
                            match self.active_tab {
                                Tab::BuilderBots => self.builderbots
                                    .iter()
                                    .map(|(id, builderbot)| html! {
                                        <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                                            <builderbot::Card key=id.clone() instance=builderbot.clone() parent=self.link.clone() />
                                        </div>
                                    }).collect::<Html>(),
                                Tab::Drones => self.drones
                                    .iter()
                                    .map(|(id, drone)| html! {
                                        <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                                            <drone::Card key=id.clone() instance=drone.clone() parent=self.link.clone() />
                                        </div>
                                    }).collect::<Html>(),
                                Tab::PiPucks => self.pipucks
                                    .iter()
                                    .map(|(id, pipuck)| html! {
                                        <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                                            <pipuck::Card key=id.clone() instance=pipuck.clone() parent=self.link.clone() />
                                        </div>
                                    }).collect::<Html>(),
                                Tab::Experiment => html! {
                                    <experiment::Interface parent=self.link.clone()
                                        builderbot_software=self.builderbot_software.clone()
                                        drone_software=self.drone_software.clone()
                                        pipuck_software=self.pipuck_software.clone() />
                                }
                            }
                        } </div>
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
