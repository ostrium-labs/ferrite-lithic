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

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="icon" type="image/svg+xml" href="/favicon.svg" />
        {/* The reveals are opacity transitions that only exist when scripting can undo
            them, so the flag has to be set before first paint. */}
        <script
          dangerouslySetInnerHTML={{
            __html: 'document.documentElement.classList.add("js")',
          }}
        />
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
      <div id="main">
        <Outlet />
      </div>
    </>
  );
}

export function ErrorBoundary({ error }: { error: unknown }) {
  const title = isRouteErrorResponse(error) ? `${error.status}` : "Something broke";
  const detail = isRouteErrorResponse(error)
    ? error.statusText || "That page is not here."
    : error instanceof Error
      ? error.message
      : "An unknown fault.";

  return (
    <main className="doc">
      <div className="doc__lede">
        <p className="eyebrow">{title}</p>
        <h1 className="doc-header__title">This page did not load</h1>
        <p>{detail}</p>
        <p>
          <a href="/">Back to the landing page</a> or <a href="/docs">the documentation index</a>.
        </p>
      </div>
    </main>
  );
}
