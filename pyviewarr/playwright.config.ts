import { defineConfig } from "@playwright/test";

export default defineConfig({
	testDir: "./tests/e2e",
	timeout: 120_000,
	expect: {
		timeout: 10_000
	},
	retries: process.env.CI ? 1 : 0,
	fullyParallel: false,
	use: {
		baseURL: "http://127.0.0.1:9999",
		trace: "on-first-retry",
		screenshot: "only-on-failure",
		video: "retain-on-failure"
	},
	webServer: {
		command:
			"jupyter lab --no-browser --ServerApp.port=9999 --ServerApp.token='' --ServerApp.password='' --ServerApp.root_dir='.' --config=tests/e2e/jupyter_server_config.py",
		url: "http://127.0.0.1:9999/lab",
		timeout: 120_000,
		reuseExistingServer: !process.env.CI
	}
});
