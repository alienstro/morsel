import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { windowLabel } from "./lib/window";
import { ReminderApp } from "./reminder/ReminderApp";
import "./styles/tokens.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {windowLabel() === "reminder" ? <ReminderApp /> : <App />}
  </React.StrictMode>,
);
