use chrono::Utc;
use ews_skill::skill::ToolResult;
use ews_skill::EwsSkill;
use serde_json::Value;

fn print_result(name: &str, result: &ToolResult) {
    if result.success {
        println!("[PASS] {}", name);
    } else {
        println!(
            "[FAIL] {}: {}",
            name,
            result.error.as_deref().unwrap_or("unknown error")
        );
    }
}

fn first_email_id(result: &ToolResult) -> Option<String> {
    let data = result.data.as_ref()?;
    let emails = data.get("emails")?.as_array()?;
    let first = emails.first()?;
    first.get("id")?.as_str().map(ToOwned::to_owned)
}

fn parse_args() -> (String, i32, Option<String>, bool) {
    let mut folder = "inbox".to_string();
    let mut limit = 10;
    let mut send_to: Option<String> = None;
    let mut do_write = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--folder" => {
                if let Some(value) = args.next() {
                    folder = value;
                }
            }
            "--limit" => {
                if let Some(value) = args.next() {
                    if let Ok(parsed) = value.parse::<i32>() {
                        limit = parsed;
                    }
                }
            }
            "--send-to" => {
                send_to = args.next();
            }
            "--do-write" => {
                do_write = true;
            }
            _ => {}
        }
    }

    (folder, limit, send_to, do_write)
}

fn print_json(label: &str, value: &Option<Value>) {
    if let Some(v) = value {
        match serde_json::to_string_pretty(v) {
            Ok(s) => println!("{}\n{}", label, s),
            Err(_) => println!("{}\n<failed to format json>", label),
        }
    }
}

fn main() {
    let (folder, limit, send_to, do_write) = parse_args();

    println!("== EWS smoke test ==");
    println!("folder={folder} limit={limit} do_write={do_write}");

    let skill = match EwsSkill::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[FATAL] failed to initialize EwsSkill: {e}");
            std::process::exit(2);
        }
    };

    let sync = skill.sync();
    print_result("sync", &sync);

    let folders = skill.list_folders();
    print_result("list_folders", &folders);

    let list = skill.list_emails(Some(folder.clone()), Some(limit), Some(false));
    print_result("list_emails", &list);
    print_json("emails payload:", &list.data);

    if let Some(email_id) = first_email_id(&list) {
        let read = skill.read_email(email_id.clone());
        print_result("read_email(first)", &read);

        let mark = skill.mark_read(email_id.clone(), true);
        print_result("mark_read(first,true)", &mark);

        let mark_back = skill.mark_read(email_id, false);
        print_result("mark_read(first,false)", &mark_back);
    } else {
        println!("[INFO] no emails found in folder '{folder}'");
    }

    let unread = skill.get_unread(Some(folder.clone()), Some(limit));
    print_result("get_unread", &unread);

    let search = skill.search("test".to_string(), Some(5));
    print_result("search('test')", &search);

    if do_write {
        if let Some(to) = send_to {
            let subject = format!("EWS smoke test {}", Utc::now().to_rfc3339());
            let body = "This is an automated smoke test email from ews-skill.".to_string();
            let send = skill.send(to, subject, body);
            print_result("send_email", &send);
        } else {
            println!("[INFO] --do-write set but --send-to missing, skipping send_email");
        }
    }

    println!("== smoke test complete ==");
}
