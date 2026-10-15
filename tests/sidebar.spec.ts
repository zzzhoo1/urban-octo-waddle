import { test, expect } from "@playwright/test";

test("sidebar renders and supports collapsed state", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("navigation", { name: "主导航" })).toBeVisible();
  await expect(page.getByText("AI 网关")).toBeVisible();

  const toggle = page.getByRole("button", { name: /打开导航|切换导航/ }).first();
  if (await toggle.isVisible()) {
    await toggle.click();
  }

  const navItems = ["仪表盘", "账号管理", "密钥分发", "模型中心", "用量统计", "调用日志"];
  for (const item of navItems) {
    await expect(page.getByRole("link", { name: item, exact: true }).first()).toBeVisible();
  }

  // collapse state should still render the action area
  await page.getByRole("button", { name: /切换/ }).nth(0).click();
  await expect(page.getByRole("button", { name: /退出登录/ }).first()).toBeVisible();
});

test("mobile sidebar opens as dialog", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");

  await page.getByRole("button", { name: "打开导航" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("移动导航")).toBeVisible();
});
