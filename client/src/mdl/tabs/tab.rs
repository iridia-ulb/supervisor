use yew::prelude::*;
use yewtil::NeqAssign;

#[derive(Clone, Debug, PartialEq, Properties)]
pub struct Props {
    #[prop_or_default]
    pub class: Classes,
    pub target: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Debug)]
pub struct Tab {
    props: Props,
    node_ref: NodeRef,
}

impl Component for Tab {
    type Message = ();
    type Properties = Props;

    fn create(props: Self::Properties, _link: ComponentLink<Self>) -> Self {
        Tab { props, node_ref: NodeRef::default() }
    }

    fn update(&mut self, _msg: Self::Message) -> ShouldRender {
        false
    }

    fn change(&mut self, props: Self::Properties) -> ShouldRender {
        self.props.neq_assign(props)
    }

    fn view(&self) -> Html {
        html! {
            <a ref=self.node_ref.clone()
               href=self.props.target
               class=classes!("mdl-tabs__tab", self.props.class.clone())>
                <i class=classes!("material-icons")>{ self.props.icon }</i>
                <span>{ self.props.label }</span>
            </a>
        }
    }

    fn rendered(&mut self, _first_render: bool) {
        if let Some(element) = self.node_ref.cast::<yew::web_sys::HtmlElement>() {
            crate::mdl::upgrade_element(element);
        }
    }
}