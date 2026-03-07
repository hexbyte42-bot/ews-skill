use chrono::Utc;
use ews_skill::skill::ToolResult;
use ews_skill::EwsSkill;
use serde_json::Value;
use std::thread::sleep;
use std::time::Duration;

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

fn parse_args() -> (String, i32, Option<String>, bool, bool) {
    let mut folder = "inbox".to_string();
    let mut limit = 10;
    let mut send_to: Option<String> = None;
    let mut do_write = false;
    let mut test_delete_modes = false;

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
            "--test-delete-modes" => {
                test_delete_modes = true;
            }
            _ => {}
        }
    }

    (folder, limit, send_to, do_write, test_delete_modes)
}

fn print_json(label: &str, value: &Option<Value>) {
    if let Some(v) = value {
        match serde_json::to_string_pretty(v) {
            Ok(s) => println!("{}\n{}", label, s),
            Err(_) => println!("{}\n<failed to format json>", label),
        }
    }
}

fn email_entries(result: &ToolResult) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(data) = result.data.as_ref() else {
        return out;
    };
    let Some(emails) = data.get("emails").and_then(|v| v.as_array()) else {
        return out;
    };

    for email in emails {
        let id = email
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let subject = email
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !id.is_empty() {
            out.push((id, subject));
        }
    }

    out
}

fn find_email_id_by_subject(
    skill: &EwsSkill,
    folder: &str,
    subject: &str,
    limit: i32,
    attempts: usize,
) -> Option<String> {
    for _ in 0..attempts {
        let list = skill.list_emails(Some(folder.to_string()), Some(limit), Some(false));
        for (id, s) in email_entries(&list) {
            if s == subject {
                return Some(id);
            }
        }
        let _ = skill.sync();
        sleep(Duration::from_secs(2));
    }
    None
}

fn subject_exists(skill: &EwsSkill, folder: &str, subject: &str, limit: i32) -> bool {
    let list = skill.list_emails(Some(folder.to_string()), Some(limit), Some(false));
    email_entries(&list).iter().any(|(_, s)| s == subject)
}

fn main() {
    let (folder, limit, send_to, do_write, test_delete_modes) = parse_args();

    println!("== EWS smoke test ==");
    println!(
        "folder={folder} limit={limit} do_write={do_write} test_delete_modes={test_delete_modes}"
    );

    let skill = match EwsSkill::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[FATAL] failed to initialize EwsSkill: {e}");
            std::process::exit(2);
        }
    };

    let sync = skill.sync();
    print_result("sync", &sync);

    let server_folders = skill.list_server_folders();
    print_result("list_server_folders", &server_folders);

    let synced_folders = skill.list_synced_folders();
    print_result("list_synced_folders", &synced_folders);

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

    let search = skill.search(
        Some("test".to_string()),
        None,
        None,
        None,
        None,
        None,
        Some(5),
        Some(true),
    );
    print_result("search('test')", &search);

    if do_write {
        let resolved_send_to = send_to.or_else(|| std::env::var("EWS_EMAIL").ok());
        if let Some(to) = resolved_send_to {
            let subject = format!("EWS smoke test {}", Utc::now().to_rfc3339());
            let body = "This is an automated smoke test email from ews-skill.".to_string();
            let send = skill.send(to.clone(), subject, body);
            print_result("send_email", &send);

            if test_delete_modes {
                print_result(
                    "add_folder(deleteditems)",
                    &skill.add_folder("deleteditems".to_string()),
                );
                print_result("sync(before_delete_modes)", &skill.sync());

                let token = format!("{}", Utc::now().timestamp_millis());
                let subject_default = format!("smoke-delete-default-{token}");
                let subject_soft = format!("smoke-delete-soft-{token}");

                print_result(
                    "send_email(delete_default_case)",
                    &skill.send(
                        to.clone(),
                        subject_default.clone(),
                        "smoke delete default case".to_string(),
                    ),
                );
                print_result(
                    "send_email(delete_soft_case)",
                    &skill.send(
                        to,
                        subject_soft.clone(),
                        "smoke delete soft case".to_string(),
                    ),
                );

                print_result("sync(after_send_delete_cases)", &skill.sync());

                let maybe_default_id =
                    find_email_id_by_subject(&skill, "inbox", &subject_default, 200, 10);
                let maybe_soft_id =
                    find_email_id_by_subject(&skill, "inbox", &subject_soft, 200, 10);

                if let (Some(default_id), Some(soft_id)) = (maybe_default_id, maybe_soft_id) {
                    print_result(
                        "delete_default(move_to_deleteditems)",
                        &skill.delete(default_id, false),
                    );
                    print_result("delete_soft(skip_trash=true)", &skill.delete(soft_id, true));

                    print_result("sync(after_delete_cases)", &skill.sync());
                    sleep(Duration::from_secs(2));
                    print_result("sync(after_delete_cases_2)", &skill.sync());

                    let default_in_inbox = subject_exists(&skill, "inbox", &subject_default, 200);
                    let soft_in_inbox = subject_exists(&skill, "inbox", &subject_soft, 200);
                    let default_in_deleteditems =
                        subject_exists(&skill, "deleteditems", &subject_default, 200);
                    let soft_in_deleteditems =
                        subject_exists(&skill, "deleteditems", &subject_soft, 200);

                    println!(
                        "[CHECK] delete default in inbox={} deleteditems={}",
                        default_in_inbox, default_in_deleteditems
                    );
                    println!(
                        "[CHECK] delete soft in inbox={} deleteditems={}",
                        soft_in_inbox, soft_in_deleteditems
                    );

                    if !default_in_inbox
                        && !soft_in_inbox
                        && default_in_deleteditems
                        && !soft_in_deleteditems
                    {
                        println!("[PASS] delete_mode_behavior");
                    } else {
                        println!("[FAIL] delete_mode_behavior");
                    }
                } else {
                    println!(
                        "[FAIL] delete_mode_behavior: unable to locate test messages in inbox"
                    );
                }
            }
        } else {
            println!("[INFO] --do-write set but no recipient found (--send-to or EWS_EMAIL), skipping write checks");
        }
    }

    println!("== smoke test complete ==");
}
