import Link from "next/link";

const nav = [
  { href: "/features", label: "Features" },
  { href: "/integrations", label: "Integrations" },
  { href: "/getting-started", label: "Getting started" },
  { href: "/docs", label: "Docs" },
];

export default function PublicLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <header className="border-b border-border">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
          <Link href="/" className="flex items-center gap-2 font-semibold tracking-tight">
            <span aria-hidden className="inline-block h-3 w-3 rounded-full bg-accent" />
            ControlHub
          </Link>
          <nav aria-label="Main" className="flex items-center gap-6 text-sm text-muted">
            {nav.map((n) => (
              <Link key={n.href} href={n.href} className="hover:text-text">
                {n.label}
              </Link>
            ))}
            <Link
              href="/app"
              className="rounded-md bg-accent px-3 py-1.5 font-medium text-bg hover:bg-accent/90"
            >
              Open app
            </Link>
          </nav>
        </div>
      </header>
      <main className="flex-1">{children}</main>
      <footer className="border-t border-border py-6 text-center text-xs text-muted">
        ControlHub — early development build. Not production-ready.
      </footer>
    </div>
  );
}
