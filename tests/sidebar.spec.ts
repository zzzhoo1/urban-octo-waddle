import { test, expect } from "@playwright/test";

test("sidebar renders and supports collapsed state", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("navigation", { name: "主导航" })).toBeVisible();
  await expect(page.getByText("AI 网关")).toBeVisible();

  const sideToggle = page.getByRole("button").filter({ hasText: "" }).nth(0);
  await sideToggle.click();

  await expect(page.getByRole("button", { name: /退出登录/ })).toBeVisible();
  await expect(page.getByRole("link", { name: "仪表盘" })).toBeVisible();
});

test("mobile sidebar opens as dialog", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");

  await page.getByRole("button", { name: "打开导航" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("移动导航")).toBeVisible();
});
