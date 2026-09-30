import { Suspense } from "react";
import { Onboarding } from "@/components/onboarding/Onboarding";

export default function OnboardingPage() {
  return (
    <Suspense>
      <Onboarding />
    </Suspense>
  );
}
