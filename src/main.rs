use getopts::Options;
use rand::{thread_rng, Rng};
use regex::RegexSet;
use reqwest::{get, Url};
use serde_derive::Deserialize;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// This is apparently necessary: https://github.com/reddit/reddit/issues/283
fn decode_url_entities(url: &str) -> String {
    url.replace("&amp;", "&")
}

#[derive(Deserialize)]
struct SubredditChildData {
    url: String,
    post_hint: Option<String>,
}

#[derive(Deserialize)]
struct SubredditChild {
    data: SubredditChildData,
}

#[derive(Deserialize)]
struct SubredditData {
    children: Vec<SubredditChild>,
}

#[derive(Deserialize)]
struct Subreddit {
    data: SubredditData,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut opts = Options::new();
    opts.optflag("h", "help", "Show this message.");
    opts.optopt(
        "o",
        "out",
        "Directory to keep the wallpapers [default: /tmp]",
        "PATH",
    );
    opts.optopt(
        "c",
        "cmd",
        "Command to run to change the wallpaper [default: feh --bg-fill]",
        "CMD",
    );
    let matches = opts
        .parse(&args[1..])
        .expect("failed to parse command line args");

    if matches.opt_present("h") {
        println!("SFWallpaper\nDownloads top images from given subreddits and sets a random one as the wallpaper.\n");
        println!(
            "{}",
            opts.usage(&format!("Usage: {} [options] [<subreddits>...]", args[0]))
        );
        println!(
            "Downloads top images from given subreddits and sets a random one as the wallpaper."
        );
        return;
    }

    let out = matches.opt_str("o").unwrap_or_else(|| "/tmp".to_owned());
    let cmd = matches
        .opt_str("c")
        .unwrap_or_else(|| "feh --bg-fill".to_owned());

    let subreddits = if matches.free.is_empty() {
        vec!["EarthPorn".to_owned()]
    } else {
        matches.free
    };

    let image_url_regex = Arc::new(
        RegexSet::new(&[
            r"https?://(\w+\.)?imgur.com/[[:alnum:]]+(\.[[:alnum:]]{3})?",
            r"https?://(\w+\.)?reddituploads.com/[[:alnum:]]+\?.+",
        ])
        .unwrap(),
    );

    let (tx, rx) = channel();
    let mut joins = Vec::new();
    for sub_ref in subreddits {
        let tx_clone = tx.clone();
        let out_path = PathBuf::from(&out);
        let sub = sub_ref.to_string();
        let image_url_regex = image_url_regex.clone();
        let handle = thread::spawn(move || {
            let mut out = Vec::new();
            let sub_url = format!("https://www.reddit.com/r/{}.json", &sub);
            println!("downloading sub {}", &sub);
            let mut retry_count = 3;
            let mut sleep_seconds = 5;
            let res: Subreddit = loop {
                match get(&sub_url).and_then(|mut r| r.json()) {
                    Ok(v) => break v,
                    Err(e) => {
                        println!("error getting subreddit {}: {}", &sub, e);
                    }
                };
                if retry_count == 0 {
                    return;
                }
                retry_count -= 1;
                println!("retrying in {} seconds", sleep_seconds);
                thread::sleep(Duration::from_secs(sleep_seconds));
                sleep_seconds *= 2;
            };
            println!("done downloading sub {}", &sub);

            for child in res.data.children {
                let url = decode_url_entities(&child.data.url);

                if let Err(e) = Url::parse(&url) {
                    println!("given post has invalid url: {}", e);
                    continue;
                }

                if child.data.post_hint.as_ref().map(|hint| hint.as_str()) != Some("image") {
                    if !image_url_regex.is_match(&url) {
                        println!("skipping post which is apparently not an image: {}", &url);
                        continue;
                    }
                }

                let encoded_name = base64::encode_mode(url.as_bytes(), base64::Base64Mode::UrlSafe);
                let target_path = match out_path
                    .join(encoded_name.trim_right_matches('='))
                    .into_os_string()
                    .into_string()
                {
                    Ok(v) => v,
                    Err(e) => {
                        println!("failed to convert {:?} to string", e);
                        continue;
                    }
                };

                let mut out_file = match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target_path)
                {
                    Ok(v) => v,
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::AlreadyExists {
                            println!("already have {}", url);
                            out.push(target_path);
                        } else {
                            println!("error creating {:?}: {}", target_path, e);
                        }
                        continue;
                    }
                };
                let mut content_reader = match get(&url) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("failed to download {}: {}", url, e);
                        continue;
                    }
                };
                if let Err(e) = content_reader.copy_to(&mut out_file) {
                    println!("failed to copy {} to {}: {}", url, target_path, e);
                    continue;
                }
                println!("{} -> {}", url, target_path);
                out.push(target_path);
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

    for i in 0..3 {
        let sample_path = thread_rng().choose(&samples).unwrap();
        println!("displaying {}", sample_path);
        let sub_cmd_res = match Command::new("sh")
            .arg("-c")
            .arg(format!("{} {}", cmd, sample_path))
            .spawn()
        {
            Ok(mut handle) => handle.wait(),
            Err(e) => {
                println!("failed to spawn command {:?}: {}", cmd, e);
                break;
            }
        };
        match sub_cmd_res {
            Ok(v) => {
                if !v.success() {
                    println!("sub command returned error exit status");
                    if i != 2 {
                        println!("retrying {} more", 2 - i);
                        continue;
                    }
                }
            }
            Err(e) => println!("failed to get result of sub command: {}", e),
        }
        break;
    }
}
