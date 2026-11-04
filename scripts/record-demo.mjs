// Record the Countersign demo as separate, editable clips.
//
//   node scripts/record-demo.mjs
//
// The Mattermost web client in a headless browser does not receive live
// websocket updates here, so each clip opens a fresh page after the state it
// shows already exists. That is also what makes the clips easy to edit: no
// dead time waiting for an agent.

import { chromium } from "playwright-core";
import fs from "fs";
import path from "path";

const ROOT = path.resolve(new URL("..", import.meta.url).pathname);
const OUT = "/tmp/countersign-video";
const env = Object.fromEntries(
  fs
    .readFileSync(path.join(ROOT, ".env"), "utf8")
    .split("\n")
    .filter((line) => line.includes("=") && !line.startsWith("#"))
    .map((line) => {
      const index = line.indexOf("=");
      return [line.slice(0, index), line.slice(index + 1)];
    }),
);

const MM = `http://127.0.0.1:${env.MATTERMOST_HOST_PORT || 8065}`;
const ROOM_URL = `${MM}/switch/channels/checkout-incidents`;
const VIEWER_URL = "http://127.0.0.1:4090/";
const FAIL_BRANCH = "hotfix/naive-rounding";
const REPORT =
  "@fixer fix the discounted refund total is a cent off on half-cent amounts. " +
  "The reported failing test is half_cent_charges_round_up; refunds use the " +
  "same rule and must round symmetrically.";

fs.mkdirSync(OUT, { recursive: true });

const login = await fetch(`${MM}/api/v4/users/login`, {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    login_id: env.MATTERMOST_USER,
    password: env.MATTERMOST_USER_PASSWORD,
  }),
});
const token = login.headers.get("token");
const user = await login.json();
if (!token) throw new Error("Mattermost login failed");

let cachedChannel = null;
let cachedTeam = null;

async function channelId() {
  if (cachedChannel) return cachedChannel;
  if (!cachedTeam) {
    const team = await fetch(`${MM}/api/v4/teams/name/${env.MATTERMOST_TEAM_NAME}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    cachedTeam = (await team.json()).id;
  }
  const response = await fetch(
    `${MM}/api/v4/teams/${cachedTeam}/channels/name/checkout-incidents`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  cachedChannel = (await response.json()).id;
  return cachedChannel;
}

async function posts() {
  const response = await fetch(
    `${MM}/api/v4/channels/${await channelId()}/posts?per_page=40`,
    { headers: { Authorization: `Bearer ${token}` } },
  );
  const data = await response.json();
  return Object.values(data.posts || {});
}

async function waitForPost(text, timeout = 180000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const found = (await posts()).find((post) => (post.message || "").includes(text));
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 1500));
  }
  throw new Error(`no post containing ${text}`);
}

const browser = await chromium.launch({
  executablePath: "/home/kiter/.local/bin/chromium",
  headless: true,
});

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function openPage(recording = true) {
  const context = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    recordVideo: recording
      ? { dir: OUT, size: { width: 1920, height: 1080 } }
      : undefined,
  });
  await context.addCookies([
    { name: "MMAUTHTOKEN", value: token, domain: "127.0.0.1", path: "/" },
    { name: "MMUSERID", value: user.id, domain: "127.0.0.1", path: "/" },
  ]);
  const page = await context.newPage();
  return { context, page };
}

async function openRoom(page) {
  await page.goto(ROOM_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.evaluate(() => localStorage.setItem("__landingPageSeen__", "true"));
  await page.goto(ROOM_URL, { waitUntil: "domcontentloaded", timeout: 30000 });
  await page.waitForSelector("#post_textbox", { timeout: 30000 });
  await page.evaluate(() => {
    const list = document.querySelector("#postListScrollContainer");
    if (list) list.scrollTop = list.scrollHeight;
  });
  await sleep(2500);
}

async function send(page, text) {
  await page.click("#post_textbox");
  await page.type("#post_textbox", text, { delay: 6 });
  await sleep(300);
  await page.keyboard.press("Enter");
}

async function focusPost(page, text) {
  await page.evaluate((needle) => {
    const posts = Array.from(document.querySelectorAll(".post"));
    const target = posts.find((post) => post.innerText.includes(needle));
    if (target) target.scrollIntoView({ block: "start" });
    else window.scrollTo(0, 0);
  }, text);
  await sleep(1200);
}

async function closeClip(context, page, name) {
  const video = page.video();
  const tempPath = await video.path();
  await context.close();
  const target = path.join(OUT, `${name}.webm`);
  fs.renameSync(tempPath, target);
  return target;
}

async function clip(name, prepare, holdMs = 5000) {
  const { context, page } = await openPage();
  await prepare(page);
  await sleep(holdMs);
  const file = await closeClip(context, page, name);
  console.log("clip", name);
  return file;
}

const manifest = { clips: [], failReceipt: null, receipt: null };

// 1. Ask for a verification.
manifest.clips.push({
  name: "01-request-fail",
  file: await clip(
    "01-request-fail",
    async (page) => {
      await openRoom(page);
      await send(page, `@verifier verify ${FAIL_BRANCH}`);
    },
    4500,
  ),
});

// 2. The FAIL receipt.
const fail = await waitForPost("FAIL ·");
manifest.failReceipt = (fail.message.match(/R-[A-F0-9]{6}/) || [])[0] || null;
manifest.clips.push({
  name: "02-fail-receipt",
  file: await clip(
    "02-fail-receipt",
    async (page) => {
      await openRoom(page);
      await focusPost(page, "FAIL ·");
      await sleep(2000);
    },
    6500,
  ),
});

// 3. Ask the fixer for the real fix.
manifest.clips.push({
  name: "03-request-fix",
  file: await clip(
    "03-request-fix",
    async (page) => {
      await openRoom(page);
      await send(page, REPORT);
    },
    4500,
  ),
});

// 4. The proposal.
await waitForPost("Proposed fix ·");
manifest.clips.push({
  name: "04-proposal",
  file: await clip(
    "04-proposal",
    async (page) => {
      await openRoom(page);
      await focusPost(page, "Proposed fix ·");
      await sleep(1500);
    },
    5500,
  ),
});

// 5. The PASS receipt.
const pass = await waitForPost("PASS ·", 6 * 60 * 1000);
manifest.receipt = (pass.message.match(/R-[A-F0-9]{6}/) || [])[0] || null;
if (!manifest.receipt) throw new Error("no PASS receipt id");
manifest.clips.push({
  name: "05-pass",
  file: await clip(
    "05-pass",
    async (page) => {
      await openRoom(page);
      await focusPost(page, "PASS ·");
      await sleep(2000);
    },
    6500,
  ),
});

// 6. Countersign.
manifest.clips.push({
  name: "06-countersign",
  file: await clip(
    "06-countersign",
    async (page) => {
      await openRoom(page);
      await send(page, `@verifier countersign ${manifest.receipt}`);
    },
    4500,
  ),
});

// 7. Landed.
await waitForPost("LANDED", 2 * 60 * 1000);
manifest.clips.push({
  name: "07-landed",
  file: await clip(
    "07-landed",
    async (page) => {
      await openRoom(page);
      await focusPost(page, "LANDED");
      await sleep(1500);
    },
    6500,
  ),
});

// 8. The proof ledger.
manifest.clips.push({
  name: "08-ledger",
  file: await clip(
    "08-ledger",
    async (page) => {
      await page.goto(VIEWER_URL, { waitUntil: "domcontentloaded" });
      await sleep(3000);
      await page.mouse.move(960, 540);
      await page.mouse.wheel(0, 420);
      await sleep(2500);
      await page.mouse.wheel(0, 320);
    },
    5500,
  ),
});

fs.writeFileSync(path.join(OUT, "manifest.json"), JSON.stringify(manifest, null, 2));
await browser.close();
console.log(JSON.stringify(manifest, null, 2));
