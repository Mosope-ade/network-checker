// Run `cargo build`, then `npm install` and `npm run test:browser`.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const net = require("node:net");

(async () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "netcheck-browser-"));
  const port = await new Promise((resolve) => {
    const listener = net.createServer();
    listener.listen(0, "127.0.0.1", () => {
      const port = listener.address().port;
      listener.close(() => resolve(port));
    });
  });
  const base = `http://127.0.0.1:${port}`;
  const server = spawn(
    path.resolve("target/debug/netcheck"),
    ["--port", String(port), "--database", path.join(temp, "test.db")],
    { stdio: "ignore" },
  );
  let browser;
  try {
    for (let attempt = 0; attempt < 100; attempt++) {
      try {
        if ((await fetch(`${base}/api/status`)).ok) break;
      } catch {}
      await new Promise((r) => setTimeout(r, 100));
    }
    browser = await chromium.launch({
      headless: true,
      ...(process.env.CHROME_PATH
        ? { executablePath: process.env.CHROME_PATH }
        : {}),
      args: ["--no-sandbox"],
    });
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1100 },
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning")
        errors.push(message.text());
    });
    await page.goto(base);
    await page.locator("#empty").waitFor({ state: "visible" });
    await page.getByRole("button", { name: "Add your first target" }).click();
    await page
      .getByLabel("Targets", { exact: true })
      .fill(
        "127.0.0.1, Local router\n::1, IPv6 gateway\n192.0.2.1, Remote network <img src=x onerror=alert(1)>",
      );
    await page.getByLabel("Interval (seconds)").fill("1");
    await page.getByLabel("Failures to confirm").fill("2");
    await page.getByLabel("Successes to recover").fill("2");
    await page
      .getByRole("button", { name: "Start monitoring", exact: true })
      .click();
    await page.waitForFunction(
      () => document.querySelector("#down").textContent === "1",
    );
    assert.equal(await page.locator(".target").count(), 3);
    assert.equal(await page.locator(".target img").count(), 0);
    assert.equal(await page.locator("#up").textContent(), "2");
    const barHeight = await page
      .locator(".probe:not(.lost)")
      .first()
      .evaluate((el) => el.getBoundingClientRect().height);
    assert.ok(barHeight > 3, `Latency bar should be visible, got ${barHeight}`);
    await page.screenshot({
      path: "/tmp/netcheck-desktop.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Pause Local router", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Resume Local router", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Resume Local router", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Pause Local router", exact: true })
      .waitFor();
    await page.getByRole("tab", { name: /Downtime log/ }).click();
    await page.locator("#incidents tr").first().waitFor();
    const downloaded = page.waitForEvent("download");
    await page.getByRole("link", { name: "Export full CSV" }).click();
    const download = await downloaded;
    const csv = fs.readFileSync(await download.path(), "utf8");
    assert.match(csv, /Remote network/);
    await page.getByRole("tab", { name: /Live targets/ }).click();
    await page
      .getByRole("button", { name: "Remove Local router", exact: true })
      .click();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    assert.equal(await page.locator(".target").count(), 3);
    await page
      .getByRole("button", { name: "Remove IPv6 gateway", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Remove target", exact: true })
      .click();
    await page.waitForFunction(
      () => document.querySelectorAll(".target").length === 2,
    );
    await page
      .getByRole("button", { name: "Remove Local router", exact: true })
      .click();
    await page.keyboard.press("Escape");
    await page.waitForTimeout(1200);
    assert.equal(await page.locator(".target").count(), 2);
    await page
      .getByRole("searchbox", { name: "Find a target" })
      .fill("no-such-target");
    await page.getByText("No targets match your search.").waitFor();
    await page.getByRole("searchbox", { name: "Find a target" }).fill("");
    for (const width of [320, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 1000 });
      assert.ok(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
        `Overflow at ${width}px`,
      );
    }
    await page.setViewportSize({ width: 320, height: 1000 });
    await page.screenshot({ path: "/tmp/netcheck-mobile.png", fullPage: true });
    // Validate native dialog focus and keyboard tab behavior.
    await page
      .getByRole("button", { name: "Add targets", exact: false })
      .click();
    assert.equal(
      await page
        .locator("#addresses")
        .evaluate((el) => el === document.activeElement),
      true,
    );
    await page.screenshot({
      path: "/tmp/netcheck-add-dialog.png",
      fullPage: true,
    });
    await page.keyboard.press("Escape");
    await page.getByRole("tab", { name: /Live targets/ }).focus();
    await page.keyboard.press("ArrowRight");
    assert.equal(
      await page.locator("#incidents-tab").getAttribute("aria-selected"),
      "true",
    );
    assert.deepEqual(errors, []);
    console.log(
      "Browser checks passed: bulk add, real probes, latency bars, pause/resume, CSV, deletion/cancel, escaped names, search, keyboard navigation, four viewport sizes; zero console errors.",
    );
    console.log(
      "Screenshots: /tmp/netcheck-desktop.png, /tmp/netcheck-mobile.png, /tmp/netcheck-add-dialog.png",
    );
  } finally {
    await browser?.close();
    server.kill();
    await new Promise((resolve) => server.once("exit", resolve));
    fs.rmSync(temp, { recursive: true, force: true });
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
