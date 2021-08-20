use yew::prelude::*;
use yew::services::websocket::{WebSocketService, WebSocketStatus, WebSocketTask};
use yew::services::ConsoleService;
use wasm_bindgen::prelude::*;

mod mdl;

struct Drone {
    id: String,
}

impl Drone {
    fn new(id: String) -> Self {
        Self { id }
    }

    fn update(&mut self, update: shared::drone::Update) {

    }
}

struct UserInterface {
    link: ComponentLink<Self>,
    socket: Option<WebSocketTask>,
    drones: Vec<Drone>,
}

impl UserInterface {
    fn view_drone(&self, drone: &Drone) -> Html {
        html! {
            <mdl::card::Card
                class=classes!("mdl-shadow--4dp", "mdl-cell", "mdl-cell--4-col")>
                <mdl::card::title::Title class=classes!("mdl-card--expand","mdl-color--grey-300")>
                    <mdl::card::title_text::TitleText text={ drone.id.clone() }/>
                </mdl::card::title::Title>
                <mdl::card::supporting_text::SupportingText
                    class=classes!("mdl-color-text--grey-600")>
                    { "Non dolore elit adipisicing ea reprehenderit consectetur culpa." }
                </mdl::card::supporting_text::SupportingText>
                <mdl::card::actions::Actions
                    class=classes!("mdl-card--border")>
                    <mdl::button::Button label="click" class=classes!("mdl-js-button") />
                    <mdl::button::Button label="here" class=classes!("mdl-js-button") />
                </mdl::card::actions::Actions>
            </mdl::card::Card>
        }
    }
}

#[derive(Debug)]
enum Msg {

    // TODO: handle disconnects by matching against WebSocketStatus::Closed or WebSocketStatus::Error
    Notifcation(WebSocketStatus),
    Data(Result<Vec<u8>, anyhow::Error>),
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
        Self {
            link,
            socket: match socket {
                Ok(socket) => Some(socket),
                Err(_error) => {
                    ConsoleService::log("Could not connect to socket");
                    None
                }
            },
            drones: Default::default(),
        }
    }

    // pub enum DownMessage {
    //     // broadcast, trigger by change in actual drone
    //     UpdateDrone(String, drone::Update),
    //     UpdatePiPuck(String, pipuck::Update),
    // }
    

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Msg::Data(data) => match data {
                Ok(data) => match bincode::deserialize::<shared::DownMessage>(&data) {
                    Ok(decoded) => match decoded {
                        shared::DownMessage::UpdateDrone(id, update) => {
                            match self.drones.iter_mut().find(|drone| drone.id == id) {
                                Some(drone) => {
                                    drone.update(update);
                                }
                                None => {
                                    let mut drone = Drone::new(id);
                                    drone.update(update);
                                    self.drones.push(drone);
                                }
                            }
                        },
                        shared::DownMessage::UpdatePiPuck(id, update) => {},
                    },
                    Err(error) => {
                        ConsoleService::log(&format!("1. {:?}", error));
                    }
                },
                Err(err) => {
                    ConsoleService::log(&format!("2. {:?}", err));
                },
            },
            Msg::Notifcation(notification) => {
                ConsoleService::log(&format!("{:?}", notification));
            }
        }
        true
    }


    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            <mdl::layout::Layout class=classes!("mdl-js-layout","mdl-layout--fixed-header")>
                <mdl::layout::header::Header>
                    <mdl::layout::header_row::HeaderRow class=classes!("supervisor-header")>
                        <img src={"images/drone.png"}/>
                        <span class=classes!("mdl-layout-title")>{ "Supervisor" }</span>
                    </mdl::layout::header_row::HeaderRow>
                </mdl::layout::header::Header>
                <mdl::layout::content::Content>
                    <mdl::tabs::Tabs
                        class=classes!("mdl-js-tabs")>
                        <mdl::tabs::tab_bar::TabBar>
                            <mdl::tabs::tab::Tab class=classes!("is-active")
                                target="#drones"
                                icon="settings_ethernet"
                                label="Drones" />
                            <mdl::tabs::tab::Tab
                                target="#pipucks"
                                icon="settings_ethernet"
                                label="Pi-Pucks" />
                            <mdl::tabs::tab::Tab
                                target="#experiment"
                                icon="play_arrow"
                                label="Experiment" />
                        </mdl::tabs::tab_bar::TabBar >
                        <mdl::tabs::panel::Panel id="drones" class=classes!("is-active")>
                            <mdl::grid::Grid>
                                { for self.drones.iter().map(|drone| self.view_drone(drone)) }
                            </mdl::grid::Grid>
                        </mdl::tabs::panel::Panel>
                        <mdl::tabs::panel::Panel id="pipucks">
                            <mdl::grid::Grid/>
                        </mdl::tabs::panel::Panel>
                        <mdl::tabs::panel::Panel id="experiment">
                            <mdl::grid::Grid/>
                        </mdl::tabs::panel::Panel>
                    </mdl::tabs::Tabs>
                </mdl::layout::content::Content>
            </mdl::layout::Layout>
        }
    }
}

#[wasm_bindgen]
pub fn launch() -> Result<(), JsValue> {
    yew::start_app::<UserInterface>();
    Ok(())
}
