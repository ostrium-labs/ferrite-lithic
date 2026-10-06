import {
  Links,
  Meta,
  Outlet,
  Scripts,
  ScrollRestoration,
  isRouteErrorResponse,
  useLocation,
} from "react-router";

import { SiteHeader } from "./components/SiteHeader";
import "./app.css";
import "./styles/foundation.css";
import "./styles/layout.css";
import "./styles/instruments.css";
import "./styles/landing.css";
import "./styles/docs.css";
import "./styles/brand.css";

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="icon" type="image/svg+xml" href="/favicon.svg" />
        <link rel="icon" type="image/png" sizes="32x32" href="/brand/favicon-32.png" />
        <link rel="apple-touch-icon" sizes="180x180" href="/brand/apple-touch-icon.png" />
        <meta property="og:site_name" content="Ferrite Lithic" />
        <meta property="og:image" content="https://ferrite.ostriumlabs.org/brand/og-ferrite.png" />
        <meta property="og:image:width" content="1200" />
        <meta property="og:image:height" content="630" />
        <meta property="og:image:alt" content="ferrite-lithic. Hardware you write in Rust, as real gates. One graph. Two backends. One clock." />
        <meta name="twitter:card" content="summary_large_image" />
        <meta name="twitter:image" content="https://ferrite.ostriumlabs.org/brand/og-ferrite.png" />
        <Meta />
        <Links />
      </head>
      <body>
        <a className="skip" href="#main">
          Skip to content
        </a>
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}

export default function App() {
  const { pathname } = useLocation();
  return (
    <>
      <SiteHeader pathname={pathname} />
      <main id="main" tabIndex={-1}>
        <Outlet />
      </main>
    </>
  );
}

export function HydrateFallback() {
  return <main className="doc doc--error" id="main" tabIndex={-1}><p>Loading documentation…</p></main>;
}

export function ErrorBoundary({ error }: { error: unknown }) {
  const title = isRouteErrorResponse(error) ? `${error.status}` : "Something broke";
  const detail = isRouteErrorResponse(error)
    ? error.statusText || "That page is not here."
    : error instanceof Error
      ? error.message
      : "An unknown fault.";

  return (
    <>
    <SiteHeader pathname="" />
    <main className="doc doc--error" id="main" tabIndex={-1}>
      <div className="doc__lede">
        <p className="eyebrow">{title}</p>
        <h1 className="doc-header__title">{isRouteErrorResponse(error) && error.status === 404 ? "Page not found" : "This page did not load"}</h1>
        <p>{detail}</p>
        <p>
          <a href="/">Back to the landing page</a> or <a href="/docs">the documentation index</a>.
        </p>
      </div>
    </main>
    </>
  );
}
