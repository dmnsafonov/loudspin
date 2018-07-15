#![allow(unknown_lints)]
#![warn(clippy)]

extern crate capabilities;
#[macro_use] extern crate clap;
extern crate env_logger;
#[macro_use] extern crate failure;
extern crate glob;
extern crate libc;
#[macro_use] extern crate log;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate toml;

use std::fs::*;
use std::io::prelude::*;
use std::mem::*;
use std::path::PathBuf;
use std::process::*;

use capabilities::*;
use clap::*;
use failure::{*, Error};
use glob::*;
use libc::*;
use log::Level::*;

type Result<T> = std::result::Result<T, Error>;

const CAP_DAC_OVERRIDE: u8 = 1;
const CAP_SYS_RAWIO: u8 = 17;

const CONFIG_PATH: &str = "/etc/loudspin.conf";

#[allow(non_snake_case)]
fn DEFAULT_HDPARM_PATH() -> String {
    "/sbin/hdparm".to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    devices: Vec<String>,
    #[serde(default = "DEFAULT_HDPARM_PATH")]
    hdparm_path: String,
    #[serde(skip)]
    command_arg: String
}

fn main() {
    if let Err(e) = the_main() {
        let mut first = true;;
        for i in e.causes() {
            if !first {
                eprint!(": ");
            }
            eprint!("{}", i);
            first = false;
        }
        eprintln!("");
    }
}

fn the_main() -> Result<()> {
    env_logger::init();

    let config = get_config()?;

    if log_enabled!(Debug) {
        debug!("read config:");
        for i in toml::to_string(&config)
                .context("error serializing configuration for logging")?
                .lines() {
            debug!("\t{}", i);
        }
    }

    gain_caps()?;
    debug!("set capabilities");

    for g in &config.devices {
        debug!("processing glob \"{}\"", g);
        let files = glob_with(&g, &MatchOptions {
            require_literal_separator: true,
            require_literal_leading_dot: true,
            ..MatchOptions::default()
        }).context("error listing device files")?;

        for i in files {
            let dev_filename = match i {
                Ok(x) => {
                    debug!("found device file at {}", x.to_string_lossy());
                    x
                },
                Err(e) => {
                    eprintln!("failed to list file: {}", e);
                    continue;
                }
            };

            process_devfile(&config, &dev_filename)?;
        }
    }

    Ok(())
}

fn get_config() -> Result<Config> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("loudness")
            .default_value("show")
            .value_name("LOUDNESS_LEVEL")
            .possible_values(&["quiet", "loud", "show"])
        ).get_matches();

    let mut config_file = File::open(CONFIG_PATH)
        .context("error opening the configuration file")?;
    let mut config_str = String::new();
    config_file.read_to_string(&mut config_str)
        .context("error reading from the configuration file")?;
    drop(config_file);

    let mut config: Config = toml::from_str(&config_str)
        .context("error parsing the configuration")?;

    config.command_arg = matches.value_of("loudness").unwrap().to_string();

    Ok(config)
}

fn gain_caps() -> Result<()> {
    let mut caps = Capabilities::new()
        .context("error initializing capabilities")?;

    let capset = [
        Capability::CAP_DAC_OVERRIDE,
        Capability::CAP_SYS_RAWIO
    ];
    if !caps.update(&capset, Flag::Effective, true)
            || !caps.update(&capset, Flag::Inheritable, true)
            || !caps.update(&capset, Flag::Permitted, true) {
        bail!("");
    }
    caps.apply().context("error setting capabilities")?;

    set_ambient_cap(CAP_DAC_OVERRIDE)
        .context("error setting ambient capabilities")?;
    set_ambient_cap(CAP_SYS_RAWIO)
        .context("error setting ambient capabilities")?;

    Ok(())
}

fn set_ambient_cap(cap: u8) -> Result<()> { unsafe {
    let ret = prctl(
        PR_CAP_AMBIENT,
        PR_CAP_AMBIENT_RAISE as c_ulong,
        c_ulong::from(cap),
        0,
        0
    );
    if ret == -1 {
        bail!("unable to set ambient capabilities: {}",
            std::io::Error::last_os_error());
    }
    Ok(())
}}

fn process_devfile(config: &Config, dev_filename: &PathBuf) -> Result<()> {
    let mut cmd = Command::new(&config.hdparm_path);
        cmd.arg("-M");
        if config.command_arg == "show" {
            cmd.arg(&dev_filename);
        } else {
            let hdparm_arg = translate_arg(&config)?;
            cmd.arg(hdparm_arg).arg(&dev_filename.as_os_str());
        };
        cmd.spawn().context("error calling hdparm")?
            .wait().context("error waiting for hdparm to complete")?;
    debug!("executed hdparm for {}",
        dev_filename.to_string_lossy());

    Ok(())
}

fn translate_arg(config: &Config) -> Result<String> {
    Ok(match config.command_arg.as_str() {
        "quiet" => String::from("128"),
        "loud" => String::from("254"),
        _ => bail!("wrong command argument")
    })
}