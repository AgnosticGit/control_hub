// Arguments of `just dev`: `just dev` (local), `just dev ngrok` (plus the public tunnel).
import { publicUrl } from "./public-url.mjs";

const FLAGS = new Set(["--no-web", "--release", "--no-build", "ngrok"]);

/** Parsed options; `publicUrl` is set only for `ngrok`. Throws on unknown arguments. */
export function parseDevArgs(argv, env = process.env) {
  const unknown = argv.filter((a) => !FLAGS.has(a));
  if (unknown.length) throw new Error(`unknown argument(s): ${unknown.join(" ")}. Use: just dev [ngrok]`);
  const opts = {
    web: !argv.includes("--no-web"),
    release: argv.includes("--release"),
    build: !argv.includes("--no-build"),
    publicUrl: null,
  };
  if (argv.includes("ngrok")) {
    if (!opts.web) throw new Error("ngrok exposes the web app; it cannot be combined with --no-web");
    opts.publicUrl = publicUrl(env);
    if (!opts.publicUrl) throw new Error("CONTROLHUB_PUBLIC_URL=off: `just dev ngrok` needs a public URL");
  }
  return opts;
}
