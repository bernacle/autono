import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { ViewBoundary } from "./components/view-boundary";
import "./styles.css";
const client = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchIntervalInBackground: true,
      refetchOnWindowFocus: true,
      gcTime: 10 * 60 * 1000,
    },
    mutations: { retry: false },
  },
});
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ViewBoundary>
      <QueryClientProvider client={client}>
        <App />
      </QueryClientProvider>
    </ViewBoundary>
  </React.StrictMode>,
);
