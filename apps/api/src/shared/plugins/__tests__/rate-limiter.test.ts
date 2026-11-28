/**
 * Global rate limiter, composed as in `app-core.ts`: behind `requestLogger`,
 * `metricsMiddleware` and `globalErrorHandler`. The request over the limit
 * must answer 429 with the standard error envelope and `Retry-After`, never
 * escape to the error handler as a 500.
 */
import { randomUUID } from "node:crypto";
import { Elysia } from "elysia";
import { describe, expect, it } from "vitest";
import { metricsMiddleware } from "../../../middleware/metrics.middleware";
import { globalErrorHandler } from "../error-handler";
import { rateLimiter } from "../rate-limiter";
import { requestLogger } from "../request-logger";

const WINDOW_MS = 60_000;

function appWithLimit(max: number) {
	return new Elysia()
		.use(requestLogger)
		.use(metricsMiddleware)
		.use(globalErrorHandler)
		.use(rateLimiter({ windowMs: WINDOW_MS, max, prefix: randomUUID() }))
		.get("/ping", () => ({ success: true, data: "pong" }))
		.get("/refuse", ({ set }) => {
			set.status = 403;
			return { success: false, error: "no", code: "FORBIDDEN" };
		});
}

function get(
	app: ReturnType<typeof appWithLimit>,
	path: string,
	client: string,
) {
	return app.handle(
		new Request(`http://localhost${path}`, {
			headers: { "x-forwarded-for": client },
		}),
	);
}

describe("global rate limiter", () => {
	it("answers the request over the limit with 429, the envelope and Retry-After", async () => {
		const app = appWithLimit(3);
		for (let request = 1; request <= 3; request++) {
			const allowed = await get(app, "/ping", "10.0.0.1");
			expect(allowed.status, `request ${request}`).toBe(200);
		}

		const limited = await get(app, "/ping", "10.0.0.1");

		expect(limited.status).toBe(429);
		expect(await limited.json()).toEqual({
			success: false,
			error: "Demasiadas requests. Intente en 60 segundos.",
			code: "RATE_LIMITED",
		});
		const retryAfter = Number(limited.headers.get("retry-after"));
		expect(Number.isInteger(retryAfter)).toBe(true);
		expect(retryAfter).toBeGreaterThanOrEqual(1);
		expect(retryAfter).toBeLessThanOrEqual(WINDOW_MS / 1000);
		expect(limited.headers.get("x-ratelimit-limit")).toBe("3");
		expect(limited.headers.get("x-ratelimit-remaining")).toBe("0");
	});

	it("keeps answering 429 for the rest of the window", async () => {
		const app = appWithLimit(1);
		expect((await get(app, "/ping", "10.0.0.2")).status).toBe(200);
		for (let request = 0; request < 3; request++) {
			const limited = await get(app, "/ping", "10.0.0.2");
			expect(limited.status).toBe(429);
			expect(limited.headers.get("retry-after")).not.toBeNull();
		}
	});

	it("leaves responses under the limit unchanged, with the rate limit headers", async () => {
		const app = appWithLimit(5);
		const ok = await get(app, "/ping", "10.0.0.3");
		const refused = await get(app, "/refuse", "10.0.0.3");

		expect(ok.status).toBe(200);
		expect(await ok.json()).toEqual({ success: true, data: "pong" });
		expect(ok.headers.get("x-ratelimit-limit")).toBe("5");
		expect(ok.headers.get("x-ratelimit-remaining")).toBe("4");
		expect(ok.headers.get("retry-after")).toBeNull();
		expect(refused.status).toBe(403);
		expect(await refused.json()).toEqual({
			success: false,
			error: "no",
			code: "FORBIDDEN",
		});
		expect(refused.headers.get("x-ratelimit-remaining")).toBe("3");
	});

	it("counts each client separately", async () => {
		const app = appWithLimit(1);
		expect((await get(app, "/ping", "10.0.0.4")).status).toBe(200);
		expect((await get(app, "/ping", "10.0.0.4")).status).toBe(429);
		expect((await get(app, "/ping", "10.0.0.5")).status).toBe(200);
	});
});
