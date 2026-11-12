import { useEffect, useState } from "react";

export function route() {
  const hash = window.location.hash.slice(1);
  return /^\/(home|accounts|applications|profile|support|payments)(?:\/[a-zA-Z0-9-]+)?$/.test(
    hash,
  )
    ? hash
    : "/home";
}

export function navigate(path: string) {
  window.location.hash = path;
}

export function useRoute() {
  const [current, setCurrent] = useState(route);
  useEffect(() => {
    const changed = () => {
      setCurrent(route());
      document.getElementById("main")?.focus({ preventScroll: true });
      window.scrollTo(0, 0);
    };
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return current;
}

export function login() {
  sessionStorage.setItem("pb:return", route());
  window.location.assign("/auth/customer/login");
}

export function stableKey(scope: string) {
  let key = sessionStorage.getItem(scope);
  if (!key) {
    key = crypto.randomUUID();
    sessionStorage.setItem(scope, key);
  }
  return key;
}

export function doneKey(scope: string) {
  sessionStorage.removeItem(scope);
}
