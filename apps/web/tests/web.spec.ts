import { test, expect } from "@playwright/test";
import { spawn, type ChildProcess } from "child_process";

let server: ChildProcess;

test.beforeAll(async () => {
  // Start the server
  server = spawn("bun", ["run", "serve.ts"], {
    cwd: __dirname + "/..",
    stdio: "inherit",
  });

  // Wait for server to start
  await new Promise((resolve) => setTimeout(resolve, 1000));
});

test.afterAll(async () => {
  server.kill();
});

test("web app loads without errors", async ({ page }) => {
  const errors: string[] = [];

  // Capture console errors
  page.on("console", (msg) => {
    if (msg.type() === "error") {
      errors.push(msg.text());
    }
  });

  // Capture page errors
  page.on("pageerror", (err) => {
    errors.push(err.message);
  });

  // Navigate to the app
  await page.goto("http://localhost:3000");

  // Wait for the SVG to be rendered
  await page.waitForSelector("svg", { timeout: 5000 });

  // Check that SVG exists and has content
  const svg = page.locator("#app svg");
  await expect(svg).toBeVisible();

  // Check for any path elements (shapes)
  const paths = page.locator("#app svg path");
  const pathCount = await paths.count();
  expect(pathCount).toBeGreaterThan(0);

  // Log any errors for debugging
  if (errors.length > 0) {
    console.log("Errors encountered:", errors);
  }

  // Fail if there were any errors
  expect(errors).toHaveLength(0);
});

test("selection works", async ({ page }) => {
  await page.goto("http://localhost:3000");
  await page.waitForSelector("svg", { timeout: 5000 });

  // Click on a shape (the demo content has a blue rectangle around 125, 100)
  await page.click("svg", { position: { x: 125, y: 100 } });

  // Wait a moment for selection to render
  await page.waitForTimeout(100);

  // Check for selection rectangle (rect with stroke but no fill)
  const selectionRects = page.locator('svg rect[fill="none"][stroke="#0066ff"]');
  const count = await selectionRects.count();
  expect(count).toBeGreaterThan(0);
});
