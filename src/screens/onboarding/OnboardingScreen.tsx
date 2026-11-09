import { Navigate, useParams } from "react-router";
import { AiStep } from "./AiStep";
import { CalendarStep } from "./CalendarStep";
import { WelcomeStep } from "./WelcomeStep";

/** Route `/welcome/:step`: the three first-run steps. */
export function OnboardingScreen() {
  const { step } = useParams();
  if (step === "1") return <WelcomeStep />;
  if (step === "2") return <AiStep />;
  if (step === "3") return <CalendarStep />;
  return <Navigate to="/welcome/1" replace />;
}
