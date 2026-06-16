import { expect, test } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";

const serverUrl = process.env.LEMONTODO_E2E_SERVER_URL ?? "http://127.0.0.1:8787";
const email = `register-${randomUUID()}@example.com`;
const masterPassword = "dev-master-password";

test("register page creates an E2EE account usable by ltd login", async ({ page }) => {
  await page.goto(`${serverUrl}/register`);
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Master password", { exact: true }).fill(masterPassword);
  await page.getByLabel("Confirm master password").fill(masterPassword);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("status")).toContainText("Account created");

  execFileSync(
    "cargo",
    [
      "run",
      "-q",
      "-p",
      "lemontodo-tui",
      "--",
      "--db",
      `/tmp/lemontodo-register-page-${randomUUID()}.db`,
      "sync",
      "login",
      "--server-url",
      serverUrl,
      "--email",
      email,
      "--master-password",
      masterPassword,
    ],
    { stdio: "pipe" },
  );
});
