use std::time::{Instant, Duration};
use std::fs::File;
use std::io::{BufWriter, Write};
use serde::Serialize;

#[derive(Serialize)]
enum Message {
    Optitrack {},
    RobotTx {
        to: String,
        from: String,
        content: Vec<u8>,
    },
    Test {
        hello: String,
    },
}

#[derive(Serialize)]
struct Log {
    timestamp: Duration,
    message: Message,
}

fn log() -> std::io::Result<()> {
    let start = Instant::now();

    let test = Log {
        timestamp: start.elapsed(),
        message: Message::Test{ hello : format!("hello python")}
    };

    let test2 = Log {
        timestamp: start.elapsed(),
        message: Message::Test{ hello : format!("bye python")}
    };

    let output = File::create("test.pickle").expect("Unable to create file");
    let mut output = BufWriter::new(output);

    // TODO layout the pickle so that filtering by type and time is straightforward and simple
    serde_pickle::ser::to_writer(&mut output, &(1, test), true)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;

    serde_pickle::ser::to_writer(&mut output, &(2, test2), true)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;

    Ok(())
}

/* .bashrc
depickle() {
python << EOPYTHON
import pickle
f = open('${1}', 'rb')
while True:
   try:
      print(pickle.load(f))
   except EOFError:
      break
EOPYTHON
}
*/