// Vitest global setup. `@testing-library/jest-dom` extends the
// expect matchers (toBeInTheDocument, etc.) and is referenced
// from tsconfig.app.json's `types` array so TS picks up the
// augmented matchers.
import '@testing-library/jest-dom/vitest';
