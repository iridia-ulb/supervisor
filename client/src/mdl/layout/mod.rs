use yew::prelude::*;
use yewtil::NeqAssign;

pub mod content;
pub mod header_row;
pub mod header;

#[derive(Clone, Debug, PartialEq, Properties)]
pub struct Props {
    #[prop_or_default]
    pub children: Children,
    #[prop_or_default]
    pub class: Classes,
}

#[derive(Clone, Debug)]
pub struct Layout {
    props: Props,
    node_ref: NodeRef,
}

impl Component for Layout {
    type Message = ();
    type Properties = Props;

    fn create(props: Self::Properties, _link: ComponentLink<Self>) -> Self {
        Layout { props, node_ref: NodeRef::default() }
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
                 class=classes!("mdl-layout", self.props.class.clone())>
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