use std::{cell::RefCell, collections::HashMap, convert::AsRef, rc::Rc};
use shared::{DownMessage, UpMessage};
use strum::{EnumProperty, IntoEnumIterator};
use strum_macros::{AsRefStr, EnumIter, EnumProperty};
use uuid::Uuid;
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
    requests: HashMap<Uuid, Callback<Result<(), String>>>,


    drones: HashMap<String, Rc<RefCell<drone::Instance>>>,
    
    drone_config_comp: Option<ComponentLink<experiment::drone::ConfigCard>>,
    //pipuck_config_comp: Option<ComponentLink<experiment::pipuck::ConfigCard>>,
    control_config_comp: Option<ComponentLink<experiment::Interface>>,
}



pub enum Msg {

    // TODO: handle disconnects by matching against WebSocketStatus::Closed or WebSocketStatus::Error
    WebSocketNotifcation(WebSocketStatus),
    WebSocketRxData(Result<Vec<u8>, anyhow::Error>),
    
    SetActiveTab(Tab),

    SendRequest(shared::BackEndRequest, Option<Callback<Result<(), String>>>),
    

    SetDroneConfigComp(ComponentLink<experiment::drone::ConfigCard>),
    //SetPiPuckConfigComp(ComponentLink<experiment::drone::ConfigCard>),
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
            active_tab: Tab::Experiment,
            drones: Default::default(),

            requests: Default::default(),
            //pipucks: Default::default(),
            /* configuration component links */
            drone_config_comp: None,
            //pipuck_config_comp: None,
            control_config_comp: None,
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
                            shared::FrontEndRequest::AddPiPuck(_) => todo!(),
                            shared::FrontEndRequest::UpdatePiPuck(_, _) => todo!(),
                            shared::FrontEndRequest::UpdateExperiment(_) => todo!(),
                        },
                        DownMessage::Response(uuid, result) => {
                            if let Some(callback) = self.requests.remove(&uuid) {
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
                ConsoleService::log(&format!("{:?}", notification));
                false
            }
            Msg::SetDroneConfigComp(link) => {
                self.drone_config_comp = Some(link);
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
                                Tab::Drones => self.drones
                                    .values()
                                    .map(|drone| html!{
                                        <div class="column is-full-mobile is-full-tablet is-full-desktop is-half-widescreen is-one-third-fullhd">
                                            <drone::Card instance=drone parent=self.link.clone() />
                                        </div>
                                    }).collect::<Html>(),
                                Tab::PiPucks => {
                                    html! {}
                                },
                                Tab::Experiment => html! {
                                    <experiment::Interface parent=self.link.clone() />
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
