// macOS integration test: a private temporary SSH server + real Itda UI + file transfers.
// Build with `cargo build`, then PLAYWRIGHT=/path/to/playwright/index.mjs node scripts/files-e2e.mjs.
// CHROMIUM optionally selects the browser executable. No system SSH settings or user files change.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
const project = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
if (process.platform !== "darwin")
  throw new Error(
    "This SSH fixture runs on macOS. Portable file tests run with cargo test files::.",
  );
const pw = await import(
  process.env.PLAYWRIGHT
    ? pathToFileURL(process.env.PLAYWRIGHT).href
    : "playwright"
);
const root = fs.mkdtempSync(path.join(os.tmpdir(), "itda-files-test-"));
const port = async () => {
  const s = net.createServer();
  await new Promise((r) => s.listen(0, "127.0.0.1", r));
  const n = s.address().port;
  await new Promise((r) => s.close(r));
  return n;
};
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const sshPort = await port();
let appPort = await port();
let server, app, browser;
const children = [];
const hash = (p) =>
  createHash("sha256").update(fs.readFileSync(p)).digest("hex");
const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};
async function waitHTTP() {
  for (let i = 0; i < 200; i++) {
    if (app.exitCode !== null) throw new Error("Itda exited");
    try {
      const r = await fetch(`http://127.0.0.1:${appPort}/`);
      if (r.ok) return;
    } catch {}
    await delay(50);
  }
  throw new Error("Test bridge timed out");
}
try {
  for (const key of ["server", "client"]) {
    const r = spawnSync("/usr/bin/ssh-keygen", [
      "-q",
      "-t",
      "ed25519",
      "-N",
      "",
      "-f",
      path.join(root, key),
    ]);
    if (r.status) throw new Error(r.stderr.toString());
  }
  fs.writeFileSync(
    path.join(root, "sshd_config"),
    `Port ${sshPort}\nListenAddress 127.0.0.1\nHostKey ${root}/server\nAuthorizedKeysFile ${root}/client.pub\nStrictModes no\nUsePAM no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\nPidFile ${root}/sshd.pid\nSubsystem sftp internal-sftp\nLogLevel ERROR\n`,
  );
  server = spawn(
    "/usr/sbin/sshd",
    ["-D", "-e", "-f", path.join(root, "sshd_config")],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  children.push(server);
  let sshError = "";
  server.stderr.on("data", (b) => (sshError += b));
  await delay(300);
  if (server.exitCode !== null) throw new Error(sshError);
  const remote = path.join(root, "remote"),
    local = path.join(root, "local"),
    data = path.join(root, "data", "Itda");
  for (const p of [
    remote,
    local,
    data,
    path.join(remote, "Folder"),
    path.join(local, "Downloads"),
    path.join(local, "Documents"),
    path.join(local, "Desktop"),
  ])
    fs.mkdirSync(p, { recursive: true });
  fs.writeFileSync(path.join(remote, "한글 문서.txt"), "remote document");
  fs.writeFileSync(path.join(remote, "Folder", "nested.txt"), "nested");
  fs.writeFileSync(path.join(local, "upload.txt"), "local upload");
  fs.writeFileSync(path.join(local, "drag.txt"), "drag me");
  const large = "large '한글'.bin",
    bytes = Buffer.alloc(9 << 20, 17);
  fs.writeFileSync(path.join(remote, large), bytes);
  bytes.fill(29, 1 << 20, 2 << 20);
  fs.writeFileSync(path.join(local, large), bytes);
  const t = new Date(Date.now() + 5000);
  fs.utimesSync(path.join(local, large), t, t);
  fs.writeFileSync(
    path.join(data, "hosts.json"),
    JSON.stringify([
      {
        id: "files-fixture",
        name: "Studio",
        host: "127.0.0.1:5901",
        user: os.userInfo().username,
        os: "mac",
        account: "files-fixture",
        files: {
          method: "sftp",
          host: "127.0.0.1",
          port: sshPort,
          user: os.userInfo().username,
          path: remote,
        },
      },
      {
        id: "files-second",
        name: "Travel Mac",
        host: "127.0.0.1:5902",
        user: os.userInfo().username,
        os: "mac",
        account: "files-fixture",
        files: {
          method: "sftp",
          host: "127.0.0.1",
          port: sshPort,
          path: remote,
        },
      },
    ]),
  );
  fs.writeFileSync(
    path.join(data, "accounts.json"),
    JSON.stringify([
      {
        id: "files-fixture",
        name: "Test key",
        user: os.userInfo().username,
        ssh_key: path.join(root, "client"),
      },
      {
        id: "alternate",
        name: "Saved file account",
        user: os.userInfo().username,
        ssh_key: path.join(root, "client"),
      },
    ]),
  );
  fs.writeFileSync(
    path.join(data, "settings.json"),
    JSON.stringify({
      onboarded: true,
      announce: false,
      thumbnails: false,
      web: { on: false },
    }),
  );
  // Fail only the local rsync execution when requested; the remote probe still uses the
  // real server. This checks that Auto can recover through real SFTP after rsync exits.
  const bin = path.join(root, "bin"), rsyncFail = path.join(root, "fail-rsync");
  fs.mkdirSync(bin);
  fs.writeFileSync(path.join(bin, "rsync"), '#!/bin/sh\nif [ -f "$ITDA_TEST_RSYNC_FAIL" ]; then exit 23; fi\nexec /usr/bin/rsync "$@"\n', { mode: 0o755 });
  app = spawn(path.join(project, "target/debug/itda"), [], {
    cwd: project,
    env: {
      ...process.env,
      HOME: local,
      ITDA_DATA_DIR: path.join(root, "data"),
      ITDA_TEST_HTTP: String(appPort),
      ITDA_WEB_DIR: path.join(project, "web"),
      PATH: `${bin}:${process.env.PATH}`,
      ITDA_TEST_RSYNC_FAIL: rsyncFail,
    },
    stdio: "ignore",
  });
  children.push(app);
  await waitHTTP();
  browser = await pw.chromium.launch({
    headless: true,
    ...(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {}),
  });
  for (const platform of [
    "Macintosh; Intel Mac OS X 10_15_7",
    "Windows NT 10.0; Win64; x64",
  ]) {
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1000 },
      userAgent: `Mozilla/5.0 (${platform}) AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36`,
    });
    const action = async (name) => {
      const direct = page.locator(`[data-action="${name}"]`).first();
      if (await direct.isVisible()) await direct.click();
      else {
        await page.locator('[data-action="more"]').click();
        await page.locator(`.files-menu [data-action="${name}"]`).click();
      }
    };
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(
      `http://127.0.0.1:${appPort}/${platform.startsWith("Mac") ? "" : "?files=files-fixture"}`,
    );
    if (platform.startsWith("Mac")) {
      await page.locator('.card[data-id="files-fixture"]').waitFor();
      assert((await page.locator('.card [data-act="files"]').count()) === 0, "Screen cards still have file buttons");
      await page.locator('.nav-item[data-page="files"]').click();
      await page.locator(".files-home .files-network").waitFor();
      assert(await page.locator('.nav-item[data-page="files"].active').count(), "File browser nav is not selected");
      await page.locator('[data-device-card="files-second"]').waitFor();
      assert(
        (await page.locator("[data-device-card]").count()) === 2,
        "Saved network devices are missing",
      );
      await page.locator('.files-home [data-device="files-second"]').first().focus();
      await page.evaluate(() => render());
      assert(await page.evaluate(() => document.activeElement?.dataset.device === "files-second"), "Network refresh stole sidebar focus");
      await page.keyboard.press("Meta+f");
      assert(await page.locator('#search').evaluate((el) => el === document.activeElement), "Network search shortcut did not focus search");
      // WKWebView can return focus to the document when clicking a toolbar button.
      await page.evaluate(() => document.activeElement.blur());
      await page.keyboard.press("Meta+f");
      assert(await page.locator('#search').evaluate((el) => el === document.activeElement), "Document focus lost the file search shortcut");
      assert(await page.locator('.files-view-switch').isHidden(), "Network shows view controls with no alternate view");
      await page.locator("#search").fill("Travel");
      assert(
        (await page.locator("[data-device-card]").count()) === 1,
        "Network search did not filter devices",
      );
      await page.locator('#search').press('Enter');
      await page.locator('.files-setup h2').filter({ hasText: 'Travel Mac' }).waitFor();
      assert(await page.evaluate(() => Object.keys(S.sessions).length === 0), "File search started a screen session");
      await page.locator('[data-close]').click();
      await page.locator('#search').fill('unlisted-device');
      await page.locator('#search').press('Enter');
      await page.locator('.toast').filter({ hasText: '일치하는 기기가 없습니다' }).waitFor();
      await page.locator("#search").fill("");
      await page.locator('[data-action="add-device"]').click();
      await page.locator('#modals [data-key="edit"] #f-name').click();
      await page.locator('#modals [data-key="edit"] #f-name').fill("Not saved");
      await page.keyboard.press("Escape");
      await page.locator('#modals [data-key="edit"]').waitFor({ state: "detached" });
      assert((await page.locator('[data-device-card]').count()) === 2, "Cancelled editor changed the device list");
      if (process.env.SCREENSHOT_DIR) {
        fs.mkdirSync(process.env.SCREENSHOT_DIR, { recursive: true });
        await page.screenshot({ path: path.join(process.env.SCREENSHOT_DIR, 'files-browser-mac.png') });
      }
      await page
        .locator('[data-device-card="files-fixture"] [data-device]')
        .first()
        .click();
      await page.locator("[data-connect]").click();
    }
    await page.locator(".files-layer").waitFor({ timeout: 30000 });
    if (platform.startsWith("Mac")) {
      await page.locator('[data-pane="remote"] th[data-sort="kind"]').waitFor();
      assert(await page.locator('.files-tools').isHidden(), "Mac still has the extra command bar");
      assert(await page.locator('.files-locations').isHidden(), "Separate windows still look like tabs");
      const row = page.locator('[data-pane="remote"] tr[data-index]').first();
      assert(Math.abs((await row.boundingBox()).height - 22) < 1, "Mac list density differs from virtual scrolling");
      const handle = page.locator('[data-pane="remote"] [data-resize-column="0"]');
      const before = await page.locator('[data-pane="remote"] th').first().boundingBox();
      await handle.focus();
      await page.keyboard.press("ArrowRight");
      const after = await page.locator('[data-pane="remote"] th').first().boundingBox();
      assert(after.width > before.width + 5, "Column resizing did not change the column");
      const grip = await handle.boundingBox();
      await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
      await page.mouse.down();
      await page.mouse.move(grip.x + grip.width / 2 + 24, grip.y + grip.height / 2);
      await page.mouse.up();
      assert((await page.locator('[data-pane="remote"] th').first().boundingBox()).width > after.width + 15, "Column pointer resizing did not work");
      const sidebar = page.locator('.files-sidebar');
      const width = (await sidebar.boundingBox()).width;
      await page.locator('[data-resize-sidebar]').focus();
      await page.keyboard.press("ArrowRight");
      assert((await sidebar.boundingBox()).width > width, "Sidebar resizing did not work");
      await page.locator('.files-list').focus();
      await page.keyboard.press("Meta+1");
      await page.locator('.files-grid').waitFor();
      await page.keyboard.press("Meta+2");
      await page.locator('[data-pane="remote"] table').waitFor();
      await page.evaluate(() => document.activeElement.blur());
      await page.keyboard.press("Meta+Shift+g");
      await page.locator('.files-prompt [name="path"]').fill(remote);
      await page.locator('[data-choice="ok"]').click();
    }
    const folder = page
      .locator('[data-pane="remote"] [data-index]')
      .filter({ hasText: "Folder" });
    await folder.dblclick();
    await page
      .locator('[data-pane="remote"] .files-item')
      .filter({ hasText: "nested.txt" })
      .waitFor();
    await action("up");
    await page
      .locator('[data-pane="remote"] .files-item')
      .filter({ hasText: "한글 문서.txt" })
      .waitFor();
    await action("split");
    if (platform.startsWith("Mac")) {
      await page
        .locator('[data-pane="local"] .files-item')
        .filter({ hasText: "upload.txt" })
        .click();
      await action("copy");
      await page.locator('[data-side="remote"]').click();
      await action("paste");
      await page.locator('[data-choice="ok"]').click();
      await page
        .locator(".files-job")
        .filter({ hasText: "완료" })
        .waitFor({ timeout: 30000 });
      assert(
        hash(path.join(local, "upload.txt")) ===
          hash(path.join(remote, "upload.txt")),
        "SFTP copy differs",
      );
      await page
        .locator('[data-pane="remote"] .files-item')
        .filter({ hasText: "upload.txt" })
        .click();
      await action("rename");
      await page
        .locator('.files-prompt input[name="name"]')
        .fill("renamed 한글.txt");
      await page.locator('[data-choice="ok"]').click();
      await page
        .locator('[data-pane="remote"] .files-item')
        .filter({ hasText: "renamed 한글.txt" })
        .waitFor();
      await page
        .locator('[data-pane="remote"] .files-item')
        .filter({ hasText: "renamed 한글.txt" })
        .click();
      await action("trash");
      await page.locator('[data-choice="ok"]').click();
      await page.waitForFunction(
        () =>
          !document
            .querySelector('[data-pane="remote"] .files-list')
            ?.textContent.includes("renamed 한글.txt"),
      );
      await action("undo");
      await page
        .locator('[data-pane="remote"] .files-item')
        .filter({ hasText: "renamed 한글.txt" })
        .waitFor();
      assert(
        fs.existsSync(path.join(remote, "renamed 한글.txt")),
        "Undo did not restore",
      );
      const invoke = async (extra) => {
        const count = await page.locator(".files-job").count();
        await page.evaluate(
          ({ extra, local, remote }) =>
            send("files", {
              id: "files-fixture",
              op: "transfer",
              from: "local",
              source: local,
              to: "remote",
              destination: remote,
              move: false,
              policy: "replace",
              engine: "rsync",
              ...extra,
            }),
          { extra, local, remote },
        );
        await page.waitForFunction(
          (n) =>
            [...document.querySelectorAll(".files-job")].filter((e) =>
              /완료|실패|취소됨/.test(e.textContent),
            ).length > n,
          count,
          { timeout: 30000 },
        );
        const text = await page.locator(".files-job").last().textContent();
        assert(text.includes("완료"), text);
        return text;
      };
      console.log(await invoke({ names: [large] }));
      assert(
        hash(path.join(local, large)) === hash(path.join(remote, large)),
        "rsync delta copy differs",
      );
      bytes.fill(73, 2 << 20, 3 << 20);
      fs.writeFileSync(path.join(local, large), bytes);
      const changed = new Date(Date.now() + 15000);
      fs.utimesSync(path.join(local, large), changed, changed);
      fs.writeFileSync(rsyncFail, "fail the local rsync process");
      console.log(await invoke({ names: [large], engine: "auto" }));
      fs.unlinkSync(rsyncFail);
      assert(hash(path.join(local, large)) === hash(path.join(remote, large)), "Auto did not recover the failed rsync copy through SFTP");
      assert((await page.locator('.files-job').last().textContent()).startsWith("파일"), "Rsync fallback did not report SFTP");
      fs.writeFileSync(path.join(local, "move.txt"), "move with verification");
      console.log(
        await invoke({
          names: ["move.txt"],
          engine: "sftp",
          move: true,
          policy: "keep",
        }),
      );
      assert(
        !fs.existsSync(path.join(local, "move.txt")) &&
          fs.readFileSync(path.join(remote, "move.txt"), "utf8") ===
            "move with verification",
        "Move failed",
      );
      await page
        .locator('[data-pane="remote"] .files-item')
        .filter({ hasText: "move.txt" })
        .waitFor();
      const a = await page
        .locator('[data-pane="local"] .files-item')
        .filter({ hasText: "drag.txt" })
        .boundingBox();
      const b = await page
        .locator('[data-pane="remote"] .files-list')
        .boundingBox();
      await page.mouse.move(a.x + 40, a.y + 12);
      await page.mouse.down();
      await page.mouse.move(b.x + 80, b.y + b.height - 30, { steps: 12 });
      await page.mouse.up();
      await page.locator('[data-choice="ok"]').click();
      await page.waitForFunction(
        () =>
          [...document.querySelectorAll(".files-job")].some((e) =>
            e.textContent.includes("drag.txt") && e.textContent.includes("완료"),
          ),
        null,
        { timeout: 30000 },
      );
      assert(
        hash(path.join(local, "drag.txt")) ===
          hash(path.join(remote, "drag.txt")),
        "Drag copy differs",
      );
    }
    await page.waitForFunction(
      () =>
        ![...document.querySelectorAll(".files-list")].some((e) =>
          e.textContent.includes("읽는 중"),
        ),
    );
    if (process.env.SCREENSHOT_DIR) {
      fs.mkdirSync(process.env.SCREENSHOT_DIR, { recursive: true });
      await page.screenshot({
        path: path.join(
          process.env.SCREENSHOT_DIR,
          platform.startsWith("Mac") ? "files-mac.png" : "files-windows.png",
        ),
      });
    }
    // Network locations can be visited without losing the current device's browsing state.
    const previousPath = await page
      .locator('[data-path="remote"]')
      .inputValue();
    await page.locator('[data-action="network"]').first().click();
    if (platform.startsWith("Mac")) {
      await page.evaluate(() => document.activeElement.blur());
      await page.keyboard.press("Meta+f");
      assert(await page.locator('.files-layer [data-search]').evaluate((el) => el === document.activeElement), "File-window search fell through to home");
      await page.locator('.files-layer [data-action="add-device"]').click();
      await page.locator('#modals [data-key="edit"] #f-name').click();
      await page.keyboard.press('Escape');
      await page.locator('#modals [data-key="edit"]').waitFor({ state: 'detached' });
    }
    assert(
      (await page.locator(".files-layer [data-device-card]").count()) === 2,
      "Network roster lost a device",
    );
    const previousStatus = await page.evaluate(() => ({
      windows: S.fileWindows,
      sessions: S.sessions,
    }));
    await page.evaluate(() => {
      S.fileWindows = ["files-fixture", "files-second"];
      S.sessions = { "files-fixture": { state: "connected" } };
      ItdaFiles.refreshNetwork();
    });
    assert(
      (
        await page
          .locator('.files-layer [data-device-card="files-fixture"] .files-network-state')
          .textContent()
      ).includes("화면 연결됨"),
      "Screen session badge missing",
    );
    assert(
      (
        await page
          .locator('.files-layer [data-device-card="files-second"] .files-network-state')
          .textContent()
      ).includes("파일 창 열림"),
      "File-window badge missing",
    );
    if (platform.startsWith("Mac")) {
      // Exercise the real export IPC but keep the test fixture from opening OS Finder.
      await page.route("**/ipc", async (route) => {
        const body = JSON.parse(route.request().postData());
        if (body.cmd === "file_locations") body.reveal = false;
        await route.continue({ postData: JSON.stringify(body) });
      });
      await page.locator('.files-layer [data-action="locations"]').click();
      const links = path.join(data, "Locations", "Itda");
      for (
        let i = 0;
        i < 100 &&
        (!fs.existsSync(links) ||
          fs.readdirSync(links).filter((n) => n.endsWith(".inetloc")).length !==
            2);
        i++
      )
        await delay(50);
      const generated = fs
        .readdirSync(links)
        .filter((n) => n.endsWith(".inetloc"));
      assert(
        generated.length === 2,
        "Finder connection links were not created",
      );
      for (const n of generated) {
        assert(
          spawnSync("/usr/bin/plutil", ["-lint", path.join(links, n)])
            .status === 0,
          "Invalid Finder location plist",
        );
        const text = fs.readFileSync(path.join(links, n), "utf8");
        assert(
          text.includes("itda://files/") && !text.includes(root),
          "Finder link leaked private paths",
        );
      }
      await page.unroute("**/ipc");
      if (process.env.SCREENSHOT_DIR)
        await page.screenshot({
          path: path.join(process.env.SCREENSHOT_DIR, "network-mac.png"),
        });
    }
    await page.evaluate((previous) => {
      S.fileWindows = previous.windows;
      S.sessions = previous.sessions;
      ItdaFiles.refreshNetwork();
    }, previousStatus);
    await page
      .locator('[data-network-sidebar] [data-device="files-fixture"]')
      .click();
    assert(
      (await page.locator('[data-path="remote"]').inputValue()) ===
        previousPath,
      "Network view lost the folder path",
    );
    // Reconnecting above an existing workspace must show errors in the dialog and allow retry.
    await page.evaluate(() => ItdaFiles.open(hostById("files-fixture")));
    await page.locator("[data-close]").click();
    assert(
      await page.locator(".files-layer").count(),
      "Cancel closed the existing workspace",
    );
    await page.evaluate(() => ItdaFiles.open(hostById("files-fixture")));
    await page.locator(".files-advanced summary").click();
    await page.locator('.files-setup [name="port"]').fill("1");
    await page.locator("[data-connect]").click();
    await page.waitForFunction(() => {
      const button = document.querySelector("[data-connect]");
      return button && !button.disabled;
    });
    assert(
      (await page.locator(".files-dialog-error").textContent()).length > 0,
      "Reconnect error was hidden behind the workspace",
    );
    await page.locator('.files-setup [name="port"]').fill(String(sshPort));
    await page.locator("[data-connect]").click();
    await page.locator(".files-prompt").waitFor({ state: "detached" });
    // The file-specific saved account must work independently of the screen account.
    await page.evaluate(() =>
      send("save_host", {
        host: {
          ...hostById("files-fixture"),
          account: "unavailable-screen-account",
        },
      }),
    );
    await page.waitForFunction(
      () => hostById("files-fixture").account === "unavailable-screen-account",
    );
    await page.evaluate(() => ItdaFiles.open(hostById("files-fixture")));
    await page.locator('[name="account"]').selectOption("alternate");
    await page.locator("[data-connect]").click();
    await page.locator(".files-prompt").waitFor({ state: "detached" });
    assert(
      await page.evaluate(
        () =>
          hostById("files-fixture").account === "unavailable-screen-account" &&
          hostById("files-fixture").files.account === "alternate",
      ),
      "File account changed the screen account",
    );
    // Exercise independent-window boot with existing credentials: no user setup clicks.
    await page.evaluate(() => {
      window.ITDA_FILE_HOST = "files-fixture";
      ItdaFiles.boot();
    });
    await page.locator(".files-prompt").waitFor({ state: "detached" });
    assert(
      await page.locator("body.file-window").count(),
      "File window did not enter standalone mode",
    );
    if (process.env.SCREENSHOT_DIR && platform.startsWith("Mac")) {
      await page.screenshot({ path: path.join(process.env.SCREENSHOT_DIR, "files-window-mac.png") });
    }
    // Isolated renderer benchmark: no fabricated entries are used for filesystem actions.
    const largeRender = await page.evaluate(
      ({ remote, local }) => {
        const h = hostById("files-fixture");
        ItdaFiles.open(h);
        const entries = Array.from({ length: 20000 }, (_, i) => ({
          name: `item-${String(i).padStart(5, "0")}.txt`,
          dir: false,
          link: false,
          size: i,
          modified: 1700000000,
        }));
        const start = performance.now();
        ItdaFiles.message({
          type: "files_result",
          id: h.id,
          op: "connect",
          ok: true,
          data: { path: remote, entries, local_path: local, local_entries: [] },
        });
        return {
          ms: performance.now() - start,
          rows: document.querySelectorAll('[data-pane="remote"] [data-index]')
            .length,
        };
      },
      { remote, local },
    );
    assert(
      largeRender.rows <= 160,
      `Virtual list rendered ${largeRender.rows} rows`,
    );
    await page.locator('[data-pane="remote"] .files-list').evaluate((e) => {
      e.scrollTop = e.scrollHeight;
    });
    await page.locator('[data-pane="remote"] [data-index="19999"]').waitFor();
    console.log(
      `20,000-entry renderer: ${largeRender.ms.toFixed(1)}ms, ${largeRender.rows} initial DOM rows; last item reachable`,
    );
    assert(errors.length === 0, errors.join("\n"));
    console.log(`${platform}: navigation, rendering and events passed`);
    await page.evaluate(() =>
      send("files", { id: "files-fixture", op: "disconnect" }),
    );
    await delay(200);
    await page.close();
    if (platform.startsWith("Mac")) {
      app.kill("SIGTERM");
      await new Promise((r) => app.once("exit", r));
      appPort = await port();
      app = spawn(path.join(project, "target/debug/itda"), [], {
        cwd: project,
        env: {
          ...process.env,
          HOME: local,
          ITDA_DATA_DIR: path.join(root, "data"),
          ITDA_TEST_HTTP: String(appPort),
          ITDA_WEB_DIR: path.join(project, "web"),
        },
        stdio: "ignore",
      });
      children.push(app);
      await waitHTTP();
    }
  }
  console.log(
    "PASS: real SSH/SFTP, Unicode names, copy, rename, trash/undo, rsync, verified move, pointer drag, Mac/Windows styles",
  );
} finally {
  if (browser) await browser.close();
  for (const p of children.reverse())
    if (p.exitCode === null) p.kill("SIGTERM");
  await delay(300);
  fs.rmSync(root, { recursive: true, force: true });
}
