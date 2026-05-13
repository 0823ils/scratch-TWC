use serde::Deserialize;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::collections::HashSet;

const CONCURRENT_USERS: usize = 10;

#[derive(Deserialize, Debug)]
struct StudioMember {
    id: u64,
    username: String,
}

#[derive(Deserialize, Debug, Default)]
struct ProjectStats {
    loves: Option<u64>,
    views: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct ProjectAuthor {
    id: u64,
    username: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiProject {
    id: u64,
    title: String,
    author: ProjectAuthor,
    #[serde(default)]
    stats: ProjectStats,
    // 説明文・使い方（TurboWarpリンク検出用）
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
}

async fn fetch_studio_members(client: &reqwest::Client, studio_id: u64) -> Vec<StudioMember> {
    let mut members: Vec<StudioMember> = Vec::new();
    for role in ["managers", "curators"] {
        let mut offset = 0u64;
        loop {
            let url = format!(
                "https://api.scratch.mit.edu/studios/{}/{}/?limit=40&offset={}",
                studio_id, role, offset
            );
            match client.get(&url).send().await {
                Ok(resp) if resp.status() == 200 => {
                    match resp.json::<Vec<StudioMember>>().await {
                        Ok(batch) if !batch.is_empty() => {
                            let len = batch.len() as u64;
                            members.extend(batch);
                            if len < 40 { break; }
                            offset += 40;
                        }
                        _ => break,
                    }
                }
                Ok(resp) => { eprintln!("  スタジオ {} {} HTTP {}", studio_id, role, resp.status()); break; }
                Err(e) => { eprintln!("  スタジオ {} {} エラー: {}", studio_id, role, e); break; }
            }
        }
    }
    members
}

async fn fetch_user_projects(client: &reqwest::Client, username: &str) -> Vec<ApiProject> {
    let mut projects: Vec<ApiProject> = Vec::new();
    let mut offset = 0u64;
    loop {
        let url = format!(
            "https://api.scratch.mit.edu/users/{}/projects/?limit=40&offset={}",
            username, offset
        );
        match client.get(&url).send().await {
            Ok(resp) if resp.status() == 200 => {
                match resp.json::<Vec<ApiProject>>().await {
                    Ok(batch) if !batch.is_empty() => {
                        let len = batch.len() as u64;
                        projects.extend(batch);
                        if len < 40 { break; }
                        offset += 40;
                    }
                    Ok(_) => break,
                    Err(e) => { eprintln!("  [{}] JSONパース失敗: {}", username, e); break; }
                }
            }
            Ok(resp) => { eprintln!("  [{}] HTTP {}", username, resp.status()); break; }
            Err(e) => { eprintln!("  [{}] リクエスト失敗: {}", username, e); break; }
        }
    }
    projects
}

fn has_turbowarp(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("turbowarp") || lower.contains("turbo warp")
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"', "\\\"")
     .replace('\n', " ")
     .replace('\r', "")
}

#[tokio::main]
async fn main() {
    let content = tokio::fs::read_to_string("studios.txt").await
        .expect("studios.txt が見つかりません");

    let studio_ids: Vec<u64> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.parse::<u64>().ok())
        .collect();

    if studio_ids.is_empty() {
        eprintln!("studios.txt に有効なスタジオIDがありません");
        return;
    }

    println!("対象スタジオ数: {}", studio_ids.len());

    let client = Arc::new(
        reqwest::Client::builder()
            .user_agent("Mozilla/5.0 ScratchExplorer/1.0")
            .timeout(std::time::Duration::from_secs(15))
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(50)
            .build()
            .unwrap(),
    );

    let mut user_set: HashSet<(u64, String)> = HashSet::new();
    for &sid in &studio_ids {
        println!("スタジオ {} のメンバーを取得中...", sid);
        let members = fetch_studio_members(&client, sid).await;
        println!("  → {} 人", members.len());
        for m in members { user_set.insert((m.id, m.username)); }
    }

    let users: Vec<(u64, String)> = user_set.into_iter().collect();
    let total_users = users.len();
    println!("\n重複除去後のユーザー数: {} 人\n作品の取得を開始します...\n", total_users);

    let users_file = OpenOptions::new().create(true).append(true)
        .open("users.jsonl").await.expect("users.jsonl を開けません");
    let projects_file = OpenOptions::new().create(true).append(true)
        .open("scratch_projects.jsonl").await.expect("scratch_projects.jsonl を開けません");

    let users_writer = Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(512 * 1024, users_file)));
    let proj_writer  = Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(512 * 1024, projects_file)));

    use std::sync::atomic::{AtomicU64, Ordering};
    let done_users    = Arc::new(AtomicU64::new(0));
    let done_projects = Arc::new(AtomicU64::new(0));

    stream::iter(users)
        .map(|(uid, uname)| {
            let client = Arc::clone(&client);
            let uw     = Arc::clone(&users_writer);
            let pw     = Arc::clone(&proj_writer);
            let done_u = Arc::clone(&done_users);
            let done_p = Arc::clone(&done_projects);

            async move {
                let user_line = format!("{{\"id\":{},\"username\":\"{}\"}}\n", uid, esc(&uname));
                uw.lock().await.write_all(user_line.as_bytes()).await.ok();

                let projects = fetch_user_projects(&client, &uname).await;
                let proj_count = projects.len();

                {
                    let mut pw = pw.lock().await;
                    for p in &projects {
                        let author_name = p.author.username.as_deref().unwrap_or(&uname);
                        // TurboWarpリンクが説明文か使い方に含まれるか
                        let has_tw = if has_turbowarp(&p.description) || has_turbowarp(&p.instructions) { 1 } else { 0 };
                        let line = format!(
                            "{{\"id\":{},\"title\":\"{}\",\"author_id\":{},\"author_username\":\"{}\",\"loves\":{},\"views\":{},\"has_tw\":{}}}\n",
                            p.id,
                            esc(&p.title),
                            p.author.id,
                            esc(author_name),
                            p.stats.loves.unwrap_or(0),
                            p.stats.views.unwrap_or(0),
                            has_tw,
                        );
                        pw.write_all(line.as_bytes()).await.ok();
                    }
                }

                let u = done_u.fetch_add(1, Ordering::Relaxed) + 1;
                let p = done_p.fetch_add(proj_count as u64, Ordering::Relaxed) + proj_count as u64;
                println!("[{}/{}] {} → {}作品 (累計 {}作品)", u, total_users, uname, proj_count, p);
            }
        })
        .buffer_unordered(CONCURRENT_USERS)
        .for_each(|_| async {})
        .await;

    users_writer.lock().await.flush().await.ok();
    proj_writer.lock().await.flush().await.ok();

    println!(
        "\n完了！ ユーザー: {} 人 / 作品: {} 件",
        done_users.load(Ordering::Relaxed),
        done_projects.load(Ordering::Relaxed),
    );
}