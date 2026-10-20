import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
afterEach(() => cleanup());
Object.defineProperty(window, "matchMedia", {
  value: vi.fn(() => ({
    matches: true,
    addListener: vi.fn(),
    removeListener: vi.fn(),
  })),
});
Element.prototype.scrollIntoView = vi.fn();
