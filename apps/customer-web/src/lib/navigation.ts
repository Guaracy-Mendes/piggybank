import { useEffect, useState } from "react";

export function validRoute(path: string) {
  return (
    /^\/(home|accounts|applications|profile|support|payments)(?:\/[a-zA-Z0-9-]+)?$/.test(
      path,
    ) || /^\/accounts\/[a-f0-9-]{36}\/periodic\/[a-f0-9-]{36}$/.test(path)
  );
}
export function route() {
  const hash = window.location.hash.slice(1);
  return validRoute(hash) ? hash : "/home";
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
