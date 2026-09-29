import Link from "next/link";
import { Logo } from "@/components/ui/Icon";

const nav = [
  { href: "/features", label: "Features" },
  { href: "/integrations", label: "Integrations" },
  { href: "/getting-started", label: "Getting started" },
  { href: "/docs", label: "Docs" },
];

export default function PublicLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <header className="sticky top-0 z-10 border-b border-border bg-bg/70 backdrop-blur">
        <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-3.5">
          <Link href="/" className="flex items-center gap-2.5 text-[17px] font-semibold tracking-tight">
            <Logo />
            ControlHub
          </Link>
          <nav aria-label="Main" className="flex items-center gap-6 text-sm text-muted">
            {nav.map((n) => (
              <Link key={n.href} href={n.href} className="transition-colors hover:text-text">
                {n.label}
              </Link>
            ))}
            <Link href="/app" className="btn btn-primary h-9 px-4 text-sm">
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
