import { defineConfig } from "drizzle-kit";

export default defineConfig({
  dialect: "sqlite",
  schema: "./db/schema.ts",
  out: "./db/migrations",
  // The Rust runner applies these files in order; it never reads drizzle's
  // journal, so keep names stable and prefix-ordered.
  migrations: { prefix: "index" },
  dbCredentials: {
    url: './temp/app.db'
  }
});
