use ews_skill::EwsSkill;
use std::thread;
use std::time::Duration;

fn main() {
    let _skill = match EwsSkill::from_env() {
        Ok(skill) => skill,
        Err(e) => {
            eprintln!("failed to start sync daemon: {e}");
            std::process::exit(2);
        }
    };

    println!("ews-skill sync daemon running (Ctrl+C to stop)");

    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
