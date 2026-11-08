import { useEffect } from "react";

export default function App() {
  useEffect(() => {
    document.title = "Morsel";
  }, []);

  return (
    <main>
      <h1>Morsel</h1>
    </main>
  );
}
