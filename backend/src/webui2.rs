use moon::*;
use shared::{DownMsg, DroneStatus, PiPuckStatus, UpMsg};

async fn frontend() -> Frontend {
    Frontend::new()
        .title("Supervisor")
        .append_to_head(r#"
            <link rel="stylesheet" href="/_api/public/fonts.css">
            <link rel="stylesheet" href="/_api/public/icons.css">
            <link rel="stylesheet" href="/_api/public/theme.css">
            <link rel="stylesheet" href="/_api/public/styles.css">
        "#)
            .body_content(r#"
                <div class="mdl-layout__container" id="main"></div>
                <script src="/_api/public/theme.js"></script>"#
            )
}



async fn up_msg_handler(req: UpMsgRequest<UpMsg>) {
    println!("{:#?}", req);

    let UpMsgRequest { up_msg, cor_id, .. } = req;

    

    let status = DroneStatus {
        id: "Drone".to_string(),
    };
    sessions::broadcast_down_msg(&DownMsg::DroneUpdate(status), cor_id).await;

    let status = PiPuckStatus {
        id: "PiPuck".to_string(),
    };
    sessions::broadcast_down_msg(&DownMsg::PiPuckUpdate(status), cor_id).await;

    let status = PiPuckStatus {
        id: "PiPuck2".to_string(),
    };
    sessions::broadcast_down_msg(&DownMsg::PiPuckUpdate(status), cor_id).await;

    let status = PiPuckStatus {
        id: "PiPuck".to_string(),
    };
    sessions::broadcast_down_msg(&DownMsg::PiPuckUpdate(status), cor_id).await;
    
}

#[moon::main]
async fn main() -> std::io::Result<()> {
    start(frontend, up_msg_handler, |_|{}).await
}

// whenever a drone changes, we should `sessions::broadcast_down_msg` with the id of the drone
// so that it can be updated
