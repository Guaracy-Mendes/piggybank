import { useEffect, useState } from "react";

export function validRoute(path: string) {
  return (
    /^\/(save|spend|invest|wallet|preferences|notifications|documents)$/.test(
      path,
    ) ||
    /^\/save\/(new|milestones|interest|[a-f0-9-]{36})$/.test(path) ||
    /^\/spend\/activity$/.test(path) ||
    /^\/invest\/(watchlist|analytics|automate)$/.test(path) ||
    /^\/invest\/(asset|trade)\/[A-Z0-9]{1,10}$/.test(path) ||
    /^\/invest\/review\/[a-f0-9-]{36}$/.test(path) ||
    /^\/(home|accounts|applications|profile|support|payments)(?:\/[a-zA-Z0-9-]+)?$/.test(
      path,
    ) ||
    /^\/accounts\/[a-f0-9-]{36}\/(?:periodic|offers)\/[a-f0-9-]{36}$/.test(
      path,
    ) ||
    /^\/(sharing|connected\/[a-f0-9-]{36})$/.test(path)
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

/** UI-02: create a sign-in at the issuer; the password is entered only on the issuer's page. New customers then
    land on Profile & security to verify their email (UI-03) before applying. */
export function register() {
  sessionStorage.setItem("pb:return", "/profile");
  window.location.assign("/auth/customer/register");
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
