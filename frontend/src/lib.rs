use shared::{DownMsg, ExperimentStatus, UpMsg};
use zoon::{*, format, eprintln, web_sys::HtmlElement};
use std::{array, mem, net::Ipv4Addr};


#[derive(Clone, Debug)]
pub struct DroneStatus {
    id: String,
    cameras: Vec<bytes::Bytes>,
    ferbedienung_connection: Option<Ipv4Addr>,
    ferbedienung_signal: u8,
}

impl DroneStatus {
    fn new(id: String) -> Self {
        Self {
            id: id,
            cameras: Default::default(),
            ferbedienung_connection: Default::default(),
            ferbedienung_signal: Default::default(),
        }
    }

    fn update(mut self, update: shared::drone::Update) -> Self {
        match update {
            shared::drone::Update::Cameras(mut data) => 
                mem::swap(&mut self.cameras, &mut data),
            shared::drone::Update::FernbedienungConnection(mut connection) => 
                mem::swap(&mut self.ferbedienung_connection, &mut connection),
            shared::drone::Update::FernbedienungSignal(mut signal) => 
                mem::swap(&mut self.ferbedienung_signal, &mut signal),
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct PiPuckStatus {
    id: String,
    cameras: Vec<bytes::Bytes>,
    ferbedienung_connection: Option<Ipv4Addr>,
    ferbedienung_signal: u8,
}

impl PiPuckStatus {
    fn new(id: String) -> Self {
        Self {
            id: id,
            cameras: Default::default(),
            ferbedienung_connection: Default::default(),
            ferbedienung_signal: Default::default(),
        }
    }

    fn update(mut self, update: shared::pipuck::Update) -> Self {
        match update {
            shared::pipuck::Update::Cameras(mut data) => 
                mem::swap(&mut self.cameras, &mut data),
            shared::pipuck::Update::FernbedienungConnection(mut connection) => 
                mem::swap(&mut self.ferbedienung_connection, &mut connection),
            shared::pipuck::Update::FernbedienungSignal(mut signal) => 
                mem::swap(&mut self.ferbedienung_signal, &mut signal),
        }
        self
    }
}


// https://getmdl.io/started/

// ------ ------
//    Statics 
// ------ ------
struct Panel {
    title: &'static str,
    icon: &'static str,
    id: &'static str,
    renderer: fn() -> RawHtmlEl
}

const PANELS: [Panel; 3] = [
    Panel {
        title: "Drones",
        icon: "settings_ethernet",
        id: "drones",
        renderer: render_drones,
    },
    Panel {
        title: "Pi-Pucks",
        icon: "settings_ethernet",
        id: "pipucks",
        renderer: render_pipucks,
    },
    Panel {
        title: "Experiment",
        icon: "play_arrow",
        id: "experiment",
        renderer: render_experiment,
    },
];

const ACTIVE_TAB_CLASS: &str = "mdl-tabs__tab is-active";
const INACTIVE_TAB_CLASS: &str = "mdl-tabs__tab";
const ACTIVE_PANEL_CLASS: &str = "mdl-tabs__panel is-active";
const INACTIVE_PANEL_CLASS: &str = "mdl-tabs__panel";

fn root() -> RawHtmlEl {
    new_mdl_element("div")
        .attr("class", "mdl-layout mdl-js-layout mdl-layout--fixed-header")
        .children(array::IntoIter::new([
            header(),
            content(),
        ]))
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = componentHandler, js_name = upgradeElement)]
    fn upgrade_element(element: HtmlElement);
}

fn new_mdl_element(tag: &str) -> RawHtmlEl {
    RawHtmlEl::new(tag)
        .after_insert(upgrade_element)
}

fn header() -> RawHtmlEl {
    new_mdl_element("header")
        .attr("class", "mdl-layout__header")
        .child(
            new_mdl_element("div")
                .attr("class", "supervisor-header mdl-layout__header-row")
                .children(array::IntoIter::new([
                    new_mdl_element("img")
                        .attr("src", "/_api/public/images/drone.png")
                        .event_handler(move |_: events::Click| send_message()),
                    new_mdl_element("span")
                        .attr("class", "mdl-layout-title")
                        .child("Supervisor")
                ]))
        )
}

fn content() -> RawHtmlEl {
    let tabs = PANELS.iter()
        .enumerate()
        .map(|(index, panel)| {
            new_mdl_element("a")
                .attr("class", match index {
                    0 => ACTIVE_TAB_CLASS,
                    _ => INACTIVE_TAB_CLASS
                })
                .attr("href", &format!("#{}", panel.id))
                .children(array::IntoIter::new([
                    new_mdl_element("i")
                        .attr("class", "material-icons")
                        .child(panel.icon),
                    new_mdl_element("span")
                        .child(panel.title)
                ]))
        });
    let tab_bar = new_mdl_element("div")
        .attr("class", "mdl-tabs__tab-bar")
        .children(tabs);
    let panels = PANELS.iter()
        .enumerate()
        .map(|(index, panel)| new_mdl_element("div")
            .attr("id", panel.id)
            .attr("class", match index {
                0 => ACTIVE_PANEL_CLASS,
                _ => INACTIVE_PANEL_CLASS
            })
            .child((panel.renderer)())
        );
    new_mdl_element("main")
        .attr("class", "mdl-layout__content")
        .child(
            new_mdl_element("div")
                .attr("class", "mdl-tabs mdl-js-tabs")
                .children(std::iter::once(tab_bar).chain(panels))
        )
}

/// transforms MutableVec<DroneStatus> into a set of cards
fn render_drones() -> RawHtmlEl {
    let cards = drones()
        .signal_vec_cloned()
        .map(|drone| card(&drone.id));
    new_mdl_element("div")
        .attr("class", "mdl-grid")
        .children_signal_vec(cards)
}

/// transforms MutableVec<PiPuckStatus> into a set of cards
fn render_pipucks() -> RawHtmlEl {
    let cards = pipucks()
        .signal_vec_cloned()
        .map(|pipuck| card(&pipuck.id));
    new_mdl_element("div")
        .attr("class", "mdl-grid")
        .children_signal_vec(cards)
}

/// transforms Mutable<ExperimentStatus> into a set of cards
fn render_experiment() -> RawHtmlEl {
    new_mdl_element("div")
        .attr("class", "mdl-grid")
        /* render drone configuration */
        .child_signal(experiment().signal_ref(|_experiment| card("1")))
        /* render pipuck configuration */
        .child_signal(experiment().signal_ref(|_experiment| card("2")))
}

fn send_message() {
    Task::start(async {
        connection()
            .send_up_msg(UpMsg::Refresh)
            .await
            .unwrap_or_else(|error| eprintln!("Failed to send message: {:?}", error))
    });
}

fn card(title: &str) -> RawHtmlEl {
    new_mdl_element("div")
        .attr("class", "mdl-card mdl-shadow--4dp mdl-cell mdl-cell--4-col")
        .children(array::IntoIter::new([
            /* title */
            new_mdl_element("div")
                .attr("class", "mdl-card__title mdl-card--expand mdl-color--grey-300")
                .child(
                    new_mdl_element("h2")
                        .attr("class", "mdl-card__title-text")
                        .child(title)),
                        
            /* supporting text */
            new_mdl_element("div")
                .attr("class", "mdl-card__supporting-text mdl-color-text--grey-600")
                .child("Some content"),
            /* actions */
            new_mdl_element("div")
            .attr("class", "mdl-card__actions mdl-card--border")
            .child(
                new_mdl_element("a")
                    .attr("class", "mdl-button mdl-js-button")
                    .attr("href", "#")
                    .child("button text")),
        ]))
}

// ------ ------
//     Start 
// ------ ------

#[wasm_bindgen(start)]
pub fn start() {
    start_app("main", root);
    connection();
}

#[static_ref]
fn drones() -> &'static MutableVec<DroneStatus> {
    MutableVec::new()
}

#[static_ref]
fn pipucks() -> &'static MutableVec<PiPuckStatus> {
    MutableVec::new()
}

#[static_ref]
fn experiment() -> &'static Mutable<ExperimentStatus> {
    Mutable::default()
}


#[static_ref]
fn connection() -> &'static Connection<UpMsg, DownMsg> {
    Connection::new(|down_message, _| {
        match down_message {
            DownMsg::UpdateDrone(id, update) => {
                let mut lock = drones().lock_mut();
                match lock.iter().position(|drone| drone.id == id) {
                    Some(index) => {
                        let status = lock
                            .get(index)
                            .map(|inner| inner.clone().update(update))
                            .unwrap();
                        lock.set_cloned(index, status);
                    }
                    None => {
                        lock.push_cloned(DroneStatus::new(id).update(update));
                    }
                }
            },
            DownMsg::UpdatePiPuck(id, update) => {
                let mut lock = pipucks().lock_mut();
                match lock.iter().position(|pipuck| pipuck.id == id) {
                    Some(index) => {
                        let status = lock
                            .get(index)
                            .map(|inner| inner.clone().update(update))
                            .unwrap();
                        lock.set_cloned(index, status);
                    }
                    None => {
                        lock.push_cloned(PiPuckStatus::new(id).update(update));
                    }
                }
            }
        };
    })
}



// fn jumbotron() -> RawHtmlEl {
//     new_mdl_element("div")
//         .attr("class", "jumbotron")
//         .child(
//             new_mdl_element("div")
//                 .attr("class", "row")
//                 .children(array::IntoIter::new([
//                     new_mdl_element("div")
//                         .attr("class", "col-md-6")
//                         .child(
//                             new_mdl_element("h1").child("MoonZoon")
//                         ),
//                     new_mdl_element("div")
//                         .attr("class", "col-md-6")
//                         .child(
//                             action_buttons()
//                         ),
//                 ]))
//         )
// }

// fn action_buttons() -> RawHtmlEl {
//     new_mdl_element("div")
//         .attr("class", "row")
//         .children(array::IntoIter::new([
//             action_button("run", "Create 1,000 rowz111", || create_rows(1_000)),
//             action_button("runlots", "Create 10,000 rowz111", || create_rows(10_000)),
//             action_button("add", "Append 1,000 rowz111", || append_rows(1_000)),
//             action_button("update", "Update every 10th rowz111", || update_rows(10)),
//             action_button("clear", "Clear", clear_rows),
//             action_button("swaprows", "Swap Rowz", swap_rows),
//         ]))
// }

// fn action_button(
//     id: &'static str, 
//     title: &'static str, 
//     on_click: fn(),
// ) -> RawHtmlEl {
//     new_mdl_element("div")
//         .attr("class", "col-sm-6 smallpad")
//         .child(
//             new_mdl_element("button")
//                 .attr("id", id)
//                 .attr("class", "btn btn-primary btn-block")
//                 .attr("type", "button")
//                 .event_handler(move |_: events::Click| on_click())
//                 .child(
//                     title
//                 )
//         )
// }

// fn table() -> RawHtmlEl {
//     new_mdl_element("table")
//         .attr("class", "table table-hover table-striped test-data")
//         .child_signal(
//             rows_exist().map(|rows_exist| rows_exist.then(|| {
//                 new_mdl_element("tbody")
//                     .attr("id", "tbody")
//                     .children_signal_vec(
//                         rows().signal_vec_cloned().map(row)
//                     )
//             }))
//         )
// }

// fn row(row: Arc<Row>) -> RawHtmlEl {
//     let id = row.id;
//     new_mdl_element("tr")
//         .attr_signal(
//             "class",
//             selected_row().signal_ref(move |selected_id| {
//                 ((*selected_id)? == id).then(|| "danger")
//             })
//         )
//         .children(array::IntoIter::new([
//             row_id(id),
//             row_label(id, row.label.signal_cloned()),
//             row_remove_button(id),
//             new_mdl_element("td")
//                 .attr("class", "col-md-6")
//         ]))
// }

// fn row_id(id: ID) -> RawHtmlEl {
//     new_mdl_element("td")
//         .attr("class", "col-md-1")
//         .child(id)
// }

// fn row_label(id: ID, label: impl Signal<Item = String> + Unpin + 'static) -> RawHtmlEl {
//     new_mdl_element("td")
//         .attr("class", "col-md-4")
//         .child(
//             new_mdl_element("a")
//                 .event_handler(move |_: events::Click| select_row(id))
//                 .child(Text::with_signal(label))
//         )
// }

// fn row_remove_button(id: ID) -> RawHtmlEl {
//     new_mdl_element("td")
//         .attr("class", "col-md-1")
//         .child(
//             new_mdl_element("a")
//                 .event_handler(move |_: events::Click| remove_row(id))
//                 .child(
//                     new_mdl_element("span")
//                         .attr("class", "glyphicon glyphicon-remove remove")
//                         .attr("aria-hidden", "true"),
//                 )
//         )
// }


