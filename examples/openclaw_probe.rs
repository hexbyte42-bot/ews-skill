use ews_skill::EwsSkill;
use serde_json::json;

fn main() {
    let skill = match EwsSkill::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to initialize skill: {e}");
            std::process::exit(2);
        }
    };

    let tools = EwsSkill::get_tools();
    println!("registered_tools={}", tools.len());

    let health = skill.execute_tool("email_health", json!({}));
    println!("email_health={}", health.success);
    if let Some(data) = health.data {
        println!("health_payload={}", data);
    }

    let list = skill.execute_tool(
        "email_list",
        json!({ "folder_name": "inbox", "limit": 5, "unread_only": false }),
    );
    println!("email_list={}", list.success);
    if let Some(data) = list.data {
        println!("list_payload={}", data);
    }
}
