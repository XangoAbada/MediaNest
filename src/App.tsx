import { useEffect } from "react";
import { useApp } from "./stores/app";
import { Onboarding } from "./views/Onboarding";
import { AppShell } from "./components/AppShell";

export default function App() {
  const { loaded, libraryPath, loadSettings } = useApp();

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  if (!loaded) return null;
  return libraryPath ? <AppShell /> : <Onboarding />;
}
