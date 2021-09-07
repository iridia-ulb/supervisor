use shared::UpMessage;
use yew::prelude::*;
use yew::{html, ChangeData, Component, ComponentLink, Html, ShouldRender};

use shared::experiment::Request;

use crate::UserInterface as Parent;

pub struct ConfigCard {
    link: ComponentLink<Self>,
    props: Props,
}

// what if properties was just drone::Instance itself?
#[derive(Clone, Properties)]
pub struct Props {
    pub parent: ComponentLink<Parent>,
}

pub enum Msg {
}

// is it possible to just add a callback to the update method
impl Component for ConfigCard {
    type Message = Msg;
    type Properties = Props;

    fn create(props: Props, link: ComponentLink<Self>) -> Self {
        props.parent.send_message(crate::Msg::SetControlConfigComp(link.clone()));
        // if props contains a closure, I could use that to communicate with the actual instance
        ConfigCard { 
            props,
            link,
        }
    }

    // this fires when a message needs to be processed
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        //let mut drone = self.props.instance.borrow_mut();
        
        true
    }

    // this fires when the parent changes the properties of this component
    fn change(&mut self, _: Self::Properties) -> ShouldRender {
        true
    }

    // `self.link.callback(...)` can only be created with a struct that impl Component
    // `|_: ClickEvent| { Msg::Click }` can probably be stored anywhere, i.e., external to the component
    // 
    fn view(&self) -> Html {
        let start_experiment_onclick = 
            self.props.parent.callback(|_| crate::Msg::SendUpMessage(UpMessage::Experiment(Request::StartExperiment)));
        let stop_experiment_onclick = 
            self.props.parent.callback(|_| crate::Msg::SendUpMessage(UpMessage::Experiment(Request::StopExperiment)));

        html! {
            <div class="card">
                <header class="card-header">
                    <nav class="card-header-title is-shadowless has-background-white-ter level is-mobile">
                        <div class="level-left">
                            <p class="level-item subtitle is-size-4">{ "Control Panel" }</p>
                        </div>
                    </nav>
                </header>
                // <div class="card-content">
                //     // <div class="content">
                        
                //     // </div>
                // </div>
                <footer class="card-footer">
                    <a class="card-footer-item" onclick=start_experiment_onclick>{ "Start experiment" }</a>
                    <a class="card-footer-item" onclick=stop_experiment_onclick>{ "Stop experiment" }</a>
                </footer>
            </div>
        }
    }
}
