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
      <header className="sticky top-0 z-10 border-b border-border bg-bg/90 backdrop-blur-sm">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-x-6 gap-y-2 px-6 py-3">
          <Link href="/" className="flex items-center gap-2.5 text-[17px] font-bold tracking-[-0.01em]">
            <Logo />
            ControlHub
          </Link>
          <nav aria-label="Main" className="-mx-3 flex max-w-full items-center gap-1 overflow-x-auto text-sm">
            {nav.map((n) => (
              <Link key={n.href} href={n.href} className="shrink-0 rounded-md px-3 py-2 text-muted transition-colors hover:bg-surface-2 hover:text-text">
                {n.label}
              </Link>
            ))}
            <Link href="/app" className="btn btn-primary ml-2 h-9 shrink-0 px-4 text-sm">
              Open app
            </Link>
          </nav>
        </div>
      </header>
      <main className="flex-1">{children}</main>
      <footer className="border-t border-border">
        <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-4 px-6 py-8 text-sm text-muted">
          <span className="flex items-center gap-2.5">
            <Logo className="h-5 w-5" />
            <span className="font-semibold text-text">ControlHub</span>
            <span>Observability · Replay · Control</span>
          </span>
          <span>
            Early access: ControlHub is an early development build. See{" "}
            <Link href="/features" className="underline decoration-border-strong hover:text-text">
              Features
            </Link>{" "}
            for what works today.
          </span>
        </div>
      </footer>
    </div>
  );
}
