import { Route, Routes } from "react-router";

function ComingSoon() {
  return <p style={{ padding: 32 }} className="muted">Coming soon.</p>;
}

/** Screen routes. Later tasks replace the placeholders with real screens. */
export function AppRoutes() {
  return (
    <Routes>
      <Route path="/" element={<ComingSoon />} />
      <Route path="/meeting/:id" element={<ComingSoon />} />
      <Route path="/meeting/:id/writing" element={<ComingSoon />} />
      <Route path="/note/:id" element={<ComingSoon />} />
      <Route path="/settings/models" element={<ComingSoon />} />
      <Route path="*" element={<ComingSoon />} />
    </Routes>
  );
}
