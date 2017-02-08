#![feature(plugin)]
#![plugin(docopt_macros)]

extern crate base64;
extern crate docopt;
extern crate hyper;
extern crate rand;
extern crate rustc_serialize;
extern crate serde_json;

use hyper::{Client, Url};
use rand::{thread_rng, Rng};
use serde_json::Value;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::channel;
use std::thread;

docopt!(Args derive Debug, "
SFWallpaper
Downloads top images from given subreddits and sets a random one as the wallpaper.

Usage:
    sfwallpaper --help
    sfwallpaper [options] [<subreddits>...]

Options:
    -h, --help      Show this message.
    -o, --out PATH  Directory to keep the wallpapers [default: /tmp]
    -c, --cmd CMD   Command to run to change the wallpaper [default: feh --bg-fill]

If no subreddits are given, default to EarthPorn.

");

fn main() {
    let mut args: Args = Args::docopt().decode().unwrap_or_else(|e| e.exit());
    if args.arg_subreddits.len() == 0 {
        args.arg_subreddits.push("EarthPorn".to_string());
    }

    let (tx, rx) = channel();
    let mut joins = Vec::new();
    for sub_ref in args.arg_subreddits {
        let tx_clone = tx.clone();
        let out_path = PathBuf::from(&args.flag_out);
        let sub = sub_ref.to_string();
        let handle = thread::spawn(move || {
            let mut out = Vec::new();
            let client = Client::new();
            let sub_url = format!("https://www.reddit.com/r/{}.json", &sub);
            println!("downloading sub {}", &sub);
            let res = match client.get(&sub_url).send() {
                Ok(v) => v,
                Err(e) => {
                    println!("error getting subreddit {}: {}", &sub, e);
                    return;
                }
            };
            println!("done downloading sub {}", &sub);
            let res_data: Value = match serde_json::from_reader(res) {
                Ok(v) => v,
                Err(e) => {
                    println!("error parsing subreddit json: {}", e);
                    return;
                }
            };

            let ref children = res_data["data"]["children"];
            if let &Value::Array(ref items) = children {
                for child in items.iter() {
                    let ref child_data = child["data"];
                    if child_data["post_hint"].as_str() != Some("image") {
                        continue;
                    }
                    let url_data = match child_data["url"].as_str() {
                        Some(s) => s,
                        _ => continue,
                    };

                    if let Err(e) = Url::parse(url_data) {
                        println!("given post has invalid url: {}", e);
                        continue;
                    }

                    let encoded_name = base64::encode_mode(url_data.as_bytes(), base64::Base64Mode::UrlSafe);
                    let target_path = match out_path.join(encoded_name.trim_right_matches('=')).into_os_string().into_string() {
                        Ok(v) => v,
                        Err(e) => {
                            println!("failed to convert {:?} to string", e);
                            continue;
                        }
                    };

                    let mut out_file = match OpenOptions::new().write(true).create_new(true).open(&target_path) {
                        Ok(v) => v,
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::AlreadyExists {
                                println!("already have {}", url_data);
                                out.push(target_path);
                            } else {
                                println!("error creating {:?}: {}", target_path, e);
                            }
                            continue;
                        }
                    };
                    let mut content_reader = match client.get(url_data).send() {
                        Ok(v) => v,
                        Err(e) => {
                            println!("failed to download {}: {}", url_data, e);
                            continue;
                        }
                    };
                    if let Err(e) = std::io::copy(&mut content_reader, &mut out_file) {
                        println!("failed to copy {} to {}: {}", url_data, target_path, e);
                        continue;
                    }
                    println!("{} -> {}", url_data, target_path);
                    out.push(target_path);
                }
            }

            tx_clone.send(out).unwrap();
        });
        joins.push(handle);
    }

    let mut samples = Vec::new();
    for handle in joins {
        if let Err(e) = handle.join() {
            println!("download thread panicked: {:?}", e);
        }
        if let Ok(mut v) = rx.try_recv() {
            samples.append(&mut v);
        }
    }

    let sample_path = thread_rng().choose(&samples).unwrap();
    if let Err(e) = Command::new("sh").arg("-c").arg(format!("{} {}", args.flag_cmd, sample_path)).spawn() {
        println!("failed to spawn command {:?}: {}", args.flag_cmd, e);
    }
}
