# MNS Supervisor
The MNS supervisor is a piece of software for securely starting, monitoring, and stopping experiments with the MNS

## Building
To build the supervisor, you must first install and configure the Rust programming language. After this installation, the supervisor can be built by running `cargo build` from the same directory that contains `Cargo.toml`.

## Usage
The supervisor can be run directly with Cargo, i.e., `cargo run`. Currently, the supervisor is a simple CLI application that enables configuring the power on a single drone. The IP address of the Xbee attached to the drone must be known and provided on the command line as follows `cargo run -- --xbee-address W.X.Y.Z`. By default, running this command will connect to the Xbee and configure the Pixhawk to communicate with the Up Core processor board.

To switch on the power to the Up Core, you can run `cargo run -- --xbee-address W.X.Y.Z --power-upcore true`, to switch the power to the Pixhawk, you can run `cargo run -- --xbee-address W.X.Y.Z --power-pixhawk true`. The power to these systems can be switched off by replacing `true` with `false` in the aforementioned commands. 
