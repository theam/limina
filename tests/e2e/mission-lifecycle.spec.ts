import { test, expect } from "@playwright/test";
import {
  seedMission,
  seedMissionWithPlan,
  seedMissionWithArtifacts,
  seedCompletedMission,
  seedMissionWithEscalation,
  cleanupMissions,
} from "./helpers";

const API_KEY = "test-key-for-e2e";

test.beforeEach(async ({ page }) => {
  await cleanupMissions();
  // Set API key in localStorage
  await page.goto("/");
  await page.evaluate(
    (key) => localStorage.setItem("ar-api-key", key),
    API_KEY
  );
});

test.afterAll(async () => {
  await cleanupMissions();
});

// --- Homepage ---

test("homepage shows empty state when no missions exist", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("No research missions yet")).toBeVisible();
  await expect(page.getByRole("link", { name: "New Mission" }).first()).toBeVisible();
});

test("homepage lists existing missions", async ({ page }) => {
  await seedMission("m_test001", "RUNNING");
  await page.goto("/");
  await page.reload(); // Ensure fresh fetch
  await expect(page.getByText("Test mission objective")).toBeVisible();
  await expect(page.getByText("RUNNING")).toBeVisible();
});

// --- Intake Form ---

test("intake form validates required fields", async ({ page }) => {
  await page.goto("/new");
  // Try to submit empty form
  await page.getByRole("button", { name: "Launch Mission" }).click();
  // Browser validation should prevent submission (required fields)
  // Verify we're still on the form page
  await expect(page).toHaveURL(/\/new/);
});

test("intake form shows cost estimate based on template", async ({
  page,
}) => {
  await page.goto("/new");
  // Default template is search-relevance
  await expect(page.getByText("$30–$150")).toBeVisible();
  await expect(page.getByText("12–48 hours")).toBeVisible();

  // Switch to system-investigation
  await page.getByText("System Investigation").click();
  await expect(page.getByText("$20–$100")).toBeVisible();
  await expect(page.getByText("8–36 hours")).toBeVisible();
});

test("intake form creates mission and redirects", async ({ page }) => {
  await page.goto("/new");

  // Fill out the form
  await page.getByLabel("Research Objective").fill(
    "Investigate the best approach for improving search relevance for negation queries in our e-commerce product catalog system"
  );
  await page.getByLabel("Context & Baseline").fill(
    "Current system uses BM25 with vector reranking. Negation queries like 'dress not red' return red dresses."
  );
  await page.getByLabel("Success Metric").fill("Recall@10 >= 5% improvement");

  await page.getByRole("button", { name: "Launch Mission" }).click();

  // Should redirect to mission workspace
  await expect(page).toHaveURL(/\/missions\/m_/);
});

// --- Mission Workspace ---

test("workspace shows plan tab for PLAN_READY missions", async ({ page }) => {
  await seedMissionWithPlan("m_plantest");
  await page.goto("/missions/m_plantest");

  // Should show task list
  await expect(page.getByText("T001")).toBeVisible();
  await expect(page.getByText("Analyze failure modes")).toBeVisible();
  await expect(page.getByText("T002")).toBeVisible();

  // Should show Approve button
  await expect(
    page.getByRole("button", { name: "Approve Plan" })
  ).toBeVisible();
});

test("workspace shows progress tab with artifacts", async ({ page }) => {
  await seedMissionWithArtifacts("m_progresstest");
  await page.goto("/missions/m_progresstest");

  // Should auto-select progress tab for RUNNING state
  // Should show artifacts (use exact match to avoid content duplicates)
  await expect(page.getByText("H001", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("E001", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("F001", { exact: true }).first()).toBeVisible();
});

test("workspace shows warmup state when running with no artifacts", async ({
  page,
}) => {
  await seedMission("m_warmup", "RUNNING");

  // Update state to RUNNING
  const fs = await import("fs/promises");
  const path = await import("path");
  const statePath = path.join(
    process.cwd(),
    "missions/m_warmup/mission.json"
  );
  const state = JSON.parse(await fs.readFile(statePath, "utf-8"));
  state.phase = "RUNNING";
  state.planApprovedAt = new Date().toISOString();
  await fs.writeFile(statePath, JSON.stringify(state, null, 2));

  await page.goto("/missions/m_warmup");

  await expect(
    page.getByText("Research agent is reading your challenge brief")
  ).toBeVisible();
});

test("workspace shows status block with phase badge", async ({ page }) => {
  await seedMissionWithArtifacts("m_statustest");
  await page.goto("/missions/m_statustest");

  await expect(page.getByText("RUNNING")).toBeVisible();
  await expect(page.getByText("Elapsed:")).toBeVisible();
  // Check artifact counts
  await expect(page.getByText(/H:1/)).toBeVisible();
  await expect(page.getByText(/E:1/)).toBeVisible();
  await expect(page.getByText(/F:1/)).toBeVisible();
});

test("workspace shows stop button for running missions", async ({ page }) => {
  await seedMissionWithArtifacts("m_stoptest");
  await page.goto("/missions/m_stoptest");

  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
});

// --- CEO Request / Escalation ---

test("workspace shows CEO request drawer when escalation pending", async ({
  page,
}) => {
  await seedMissionWithEscalation("m_escalation");
  await page.goto("/missions/m_escalation");

  await expect(page.getByText("Agent needs your input")).toBeVisible();
  await expect(
    page.getByText("Need access to production query logs")
  ).toBeVisible();
  await expect(page.getByPlaceholder("Your response...")).toBeVisible();
  await expect(page.getByRole("button", { name: "Respond" })).toBeVisible();
});

// --- Report Tab ---

test("workspace shows report for completed missions", async ({ page }) => {
  await seedCompletedMission("m_report");
  await page.goto("/missions/m_report");

  // Should auto-select report tab for COMPLETED state
  // Click report tab to be sure
  await page.getByRole("button", { name: "report" }).click();

  await expect(page.getByText("Key Findings")).toBeVisible();
  await expect(page.getByText("F001")).toBeVisible();
  await expect(
    page.getByText("Query rewriting achieves +3.2% recall")
  ).toBeVisible();
  await expect(page.getByText("Research Summary")).toBeVisible();
  await expect(page.getByRole("button", { name: "Share Report" })).toBeVisible();
});

test("report tab shows 'still in progress' for non-completed missions", async ({
  page,
}) => {
  await seedMissionWithArtifacts("m_notdone");
  await page.goto("/missions/m_notdone");

  // Click report tab
  await page.getByRole("button", { name: "report" }).click();

  await expect(
    page.getByText("Mission still in progress")
  ).toBeVisible();
});

// --- Failed State ---

test("workspace shows failure banner with relaunch button", async ({
  page,
}) => {
  await seedMission("m_failed", "CREATED");

  // Manually set to FAILED_RECOVERABLE
  const fs = await import("fs/promises");
  const path = await import("path");
  const statePath = path.join(
    process.cwd(),
    "missions/m_failed/mission.json"
  );
  const state = JSON.parse(await fs.readFile(statePath, "utf-8"));
  state.phase = "FAILED_RECOVERABLE";
  state.failureReason = "cook process crashed";
  await fs.writeFile(statePath, JSON.stringify(state, null, 2));

  await page.goto("/missions/m_failed");

  await expect(page.getByText("Mission failed")).toBeVisible();
  await expect(page.getByText("cook process crashed")).toBeVisible();
  await expect(page.getByRole("button", { name: "Relaunch" })).toBeVisible();
});

// --- Navigation ---

test("can navigate from homepage to intake and back", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("link", { name: "New Mission" }).first().click();
  await expect(page).toHaveURL(/\/new/);

  await page.getByText("Back to missions").click();
  await expect(page).toHaveURL("/");
});

test("can navigate from mission workspace back to homepage", async ({
  page,
}) => {
  await seedMission("m_navtest", "RUNNING");
  await page.goto("/missions/m_navtest");

  await page.getByText("All missions").click();
  await expect(page).toHaveURL("/");
});

// --- Tab Switching ---

test("can switch between plan, progress, and report tabs", async ({
  page,
}) => {
  await seedCompletedMission("m_tabtest");
  await page.goto("/missions/m_tabtest");

  // Default should be report tab for COMPLETED
  await page.getByRole("button", { name: "plan" }).click();
  await expect(page.getByText("T001")).toBeVisible();

  await page.getByRole("button", { name: "progress" }).click();
  await expect(page.getByText("H001", { exact: true }).first()).toBeVisible();

  await page.getByRole("button", { name: "report" }).click();
  await expect(page.getByText("Key Findings")).toBeVisible();
});

// --- API Auth ---

test("API rejects requests without auth", async ({ request }) => {
  const res = await request.get("/api/missions");
  expect(res.status()).toBe(401);
});

test("API accepts requests with valid auth", async ({ request }) => {
  const res = await request.get("/api/missions", {
    headers: { Authorization: `Bearer ${API_KEY}` },
  });
  expect(res.status()).toBe(200);
});
