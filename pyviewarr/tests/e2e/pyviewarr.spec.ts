import { test, expect } from "@jupyterlab/galata";

const NOTEBOOK_PATH = "tests/e2e/notebooks/widget_smoke.ipynb";

test("pyviewarr widget renders and slice controls work", async ({ page }) => {
	await page.notebook.openByPath(NOTEBOOK_PATH);
	await page.notebook.runCell(0);

	const widget = page.locator(".jp-OutputArea-output .pyviewarr").first();
	await expect(widget).toBeVisible();

	const sliceLabel = widget.locator(".pyviewarr-sliceLabel").first();
	await expect(sliceLabel).toContainText("Slice: 1 / 2");

	await widget.locator('.pyviewarr-nextButton[data-axis="0"]').click();
	await expect(sliceLabel).toContainText("Slice: 2 / 2");
});
