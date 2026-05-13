import { Database } from "bun:sqlite";
import { createReadStream, existsSync } from "node:fs";
import { createInterface } from "node:readline";

const db = new Database("projects.db");
db.run("PRAGMA journal_mode = WAL;");
db.run("PRAGMA synchronous = OFF;");
db.run("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, username TEXT)");
db.run("CREATE TABLE IF NOT EXISTS projects (id INTEGER PRIMARY KEY, title TEXT, author_id INTEGER, author_username TEXT, loves INTEGER DEFAULT 0, views INTEGER DEFAULT 0, has_tw INTEGER DEFAULT 0)");

async function importUsers() {
  const fileName = "users.jsonl";
  if (!existsSync(fileName)) return;
  const rl = createInterface({ input: createReadStream(fileName), crlfDelay: Infinity });
  const insert = db.prepare("INSERT OR IGNORE INTO users (id, username) VALUES ($id, $username)");
  let batch: any[] = [];
  let count = 0;
  const flush = db.transaction((items) => { for (const item of items) insert.run(item); });
  for await (const line of rl) {
    try {
      const u = JSON.parse(line);
      batch.push({ $id: u.id, $username: u.username });
      if (++count % 10000 === 0) { flush(batch); batch = []; console.log(`--- users ${count.toLocaleString()} ---`); }
    } catch (e) {}
  }
  if (batch.length > 0) flush(batch);
  db.run("CREATE INDEX IF NOT EXISTS idx_username ON users(username)");
  console.log(`users: ${count.toLocaleString()}`);
}

async function importProjects() {
  const fileName = "scratch_projects.jsonl";
  if (!existsSync(fileName)) return;
  const rl = createInterface({ input: createReadStream(fileName), crlfDelay: Infinity });
  const insert = db.prepare("INSERT OR IGNORE INTO projects (id, title, author_id, author_username, loves, views, has_tw) VALUES ($id, $title, $author_id, $author_username, $loves, $views, $has_tw)");
  let batch: any[] = [];
  let count = 0;
  const flush = db.transaction((items) => { for (const item of items) insert.run(item); });
  for await (const line of rl) {
    try {
      const p = JSON.parse(line);
      batch.push({ $id: p.id, $title: p.title, $author_id: p.author_id, $author_username: p.author_username, $loves: p.loves ?? 0, $views: p.views ?? 0, $has_tw: p.has_tw ?? 0 });
      if (++count % 10000 === 0) { flush(batch); batch = []; console.log(`--- projects ${count.toLocaleString()} ---`); }
    } catch (e) {}
  }
  if (batch.length > 0) flush(batch);
  db.run("CREATE INDEX IF NOT EXISTS idx_title ON projects(title)");
  db.run("CREATE INDEX IF NOT EXISTS idx_proj_author ON projects(author_username)");
  db.run("CREATE INDEX IF NOT EXISTS idx_loves ON projects(loves DESC)");
  console.log(`projects: ${count.toLocaleString()}`);
}

console.log("DB updating...");
await importUsers();
await importProjects();
console.log("Server ready: http://localhost:5001");

const PAGE_SIZE = 50;

Bun.serve({
  port: 5001,
  async fetch(req) {
    const url = new URL(req.url);

    if (url.pathname === "/api/search") {
      const q = url.searchParams.get("q") || "";
      const type = url.searchParams.get("type") || "title";
      const sort = url.searchParams.get("sort") || "loves";
      const page = Math.max(1, parseInt(url.searchParams.get("page") || "1"));
      const offset = (page - 1) * PAGE_SIZE;
      const lovesMin = url.searchParams.get("loves_min");
      const lovesMax = url.searchParams.get("loves_max");
      const authorExact = url.searchParams.get("author_exact") || "";

      const conditions: string[] = [];
      const params: Record<string, any> = {};
      let paramIndex = 0;

      const col = type === "author" ? "author_username" : "title";
      if (q.length > 0) {
        const keywords = q.trim().split(/\s+/).filter(k => k.length > 0);
        for (const kw of keywords) {
          conditions.push(`${col} LIKE $p${paramIndex}`);
          params[`$p${paramIndex}`] = `%${kw}%`;
          paramIndex++;
        }
      }

      if (lovesMin !== null && lovesMin !== "") {
        conditions.push(`loves >= $p${paramIndex}`);
        params[`$p${paramIndex}`] = parseInt(lovesMin);
        paramIndex++;
      }
      if (lovesMax !== null && lovesMax !== "") {
        conditions.push(`loves <= $p${paramIndex}`);
        params[`$p${paramIndex}`] = parseInt(lovesMax);
        paramIndex++;
      }
      if (authorExact !== "") {
        conditions.push(`author_username = $p${paramIndex}`);
        params[`$p${paramIndex}`] = authorExact;
        paramIndex++;
      }

      const where = conditions.length > 0 ? `WHERE ${conditions.join(" AND ")}` : "";

      let orderBy = "ORDER BY loves DESC";
      if (sort === "newest") orderBy = "ORDER BY id DESC";
      else if (sort === "oldest") orderBy = "ORDER BY id ASC";

      const total = (db.query(`SELECT COUNT(*) as c FROM projects ${where}`).get(params) as any).c;
      const projects = db.query(`SELECT * FROM projects ${where} ${orderBy} LIMIT ${PAGE_SIZE} OFFSET ${offset}`).all(params);
      const totalPages = Math.ceil(total / PAGE_SIZE);

      return Response.json({ projects, total, page, totalPages });
    }

    return new Response(Bun.file("index.html"));
  }
});