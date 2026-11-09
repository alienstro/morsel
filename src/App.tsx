import { useEffect } from "react";
import { BrowserRouter } from "react-router";
import { AppRoutes } from "./app/routes";
import "./styles/tokens.css";

export default function App() {
  useEffect(() => {
    document.title = "Morsel";
  }, []);

  return (
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  );
}
