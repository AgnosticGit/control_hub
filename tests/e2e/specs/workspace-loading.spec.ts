import { expect, test } from "./isolated-test";
import { collectErrors, login } from "./helpers";

test("an empty map shows a loading state until its first topology arrives", async ({ page }) => {
  let releaseResponse!: () => void;
  const heldResponse = new Promise<void>((resolve) => { releaseResponse = resolve; });
  const errors = collectErrors(page);
  await page.route("**/api/v1/projects/isolation/environments/isolation-test/topology**", async (route) => {
    const response = await route.fetch();
    await heldResponse;
    await route.fulfill({ response });
  });

  try {
    await login(page, "isolation@karbird.local", "/app/isolation/isolation-test");
    const mapLoader = page.getByRole("status", { name: "Loading system map" });
    await expect(mapLoader).toBeVisible();
    releaseResponse();
    await expect(mapLoader).toHaveCount(0);
    await expect(page.getByRole("application", { name: /Service map/ })).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    releaseResponse();
  }
});
