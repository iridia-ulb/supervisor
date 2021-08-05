use yew::prelude::*;
use yewtil::NeqAssign;

#[derive(Clone, Debug, PartialEq, Properties)]
pub struct Props {
    #[prop_or_default]
    pub children: Children,
    #[prop_or_default]
    pub class: Classes,
    pub id: &'static str
}

#[derive(Clone, Debug)]
pub struct Panel {
    props: Props,
    node_ref: NodeRef,
}

impl Component for Panel {
    type Message = ();
    type Properties = Props;

    fn create(props: Self::Properties, _link: ComponentLink<Self>) -> Self {
        Panel { props, node_ref: NodeRef::default() }
    }

    fn update(&mut self, _msg: Self::Message) -> ShouldRender {
        false
    }

    fn change(&mut self, props: Self::Properties) -> ShouldRender {
        self.props.neq_assign(props)
    }

    fn view(&self) -> Html {
        html! {
            <div ref=self.node_ref.clone()
                 id=self.props.id
                 class=classes!("mdl-tabs__panel", self.props.class.clone())>
                { self.props.children.clone() }
            </div>
        }
    }

    fn rendered(&mut self, _first_render: bool) {
        if let Some(element) = self.node_ref.cast::<yew::web_sys::HtmlElement>() {
            crate::mdl::upgrade_element(element);
        }
    }
}