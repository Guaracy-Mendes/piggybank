/* Live password-strength meter for the registration page (UI-02). It only guides the person: the realm password
   policy (8+ characters, an uppercase letter, a symbol, not the email) is enforced by the issuer on submit. Nothing is
   stored or sent anywhere by this script. */
(function () {
  "use strict";
  var RULES = [
    { id: "length", text: "At least 8 characters", test: function (v) { return v.length >= 8; } },
    { id: "upper", text: "An uppercase letter", test: function (v) { return /[A-Z]/.test(v); } },
    { id: "symbol", text: "A symbol, such as ! or -", test: function (v) { return /[^A-Za-z0-9]/.test(v); } }
  ];
  function label(met, length) {
    if (met < RULES.length) return "Weak";
    return length >= 12 ? "Strong" : "Good";
  }
  function setup() {
    if (document.body.getAttribute("data-page-id") !== "login-register") return;
    var input = document.getElementById("password");
    if (!input) return;
    var form = document.getElementById("kc-register-form");
    if (form && !document.querySelector(".pb-intro")) {
      var intro = document.createElement("p");
      intro.className = "pb-intro";
      intro.textContent =
        "First, create your sign-in. Next, you choose an account and give your details in the app. Your password is only entered here, with the sign-in service.";
      form.parentNode.insertBefore(intro, form);
    }
    var box = document.createElement("div");
    box.className = "pb-strength";
    box.id = "pb-password-strength";
    var meter = document.createElement("meter");
    meter.min = 0;
    meter.max = RULES.length + 1;
    meter.low = RULES.length;
    meter.high = RULES.length + 1;
    meter.optimum = RULES.length + 1;
    meter.value = 0;
    meter.setAttribute("aria-label", "Password strength");
    var status = document.createElement("p");
    status.setAttribute("role", "status");
    status.setAttribute("aria-live", "polite");
    status.id = "pb-password-strength-text";
    var list = document.createElement("ul");
    var items = RULES.map(function (r) {
      var li = document.createElement("li");
      li.textContent = r.text;
      li.setAttribute("data-rule", r.id);
      li.setAttribute("data-met", "false");
      list.appendChild(li);
      return li;
    });
    box.appendChild(meter);
    box.appendChild(status);
    box.appendChild(list);
    var anchor = document.getElementById("input-error-container-password") || input.parentNode;
    anchor.parentNode.insertBefore(box, anchor.nextSibling);
    var described = input.getAttribute("aria-describedby");
    input.setAttribute("aria-describedby", (described ? described + " " : "") + "pb-password-strength-text");
    function update() {
      var v = input.value;
      var met = 0;
      RULES.forEach(function (r, i) {
        var ok = r.test(v);
        if (ok) met += 1;
        items[i].setAttribute("data-met", ok ? "true" : "false");
      });
      var strong = met === RULES.length && v.length >= 12;
      meter.value = v.length === 0 ? 0 : met + (strong ? 1 : 0);
      status.textContent = v.length === 0
        ? "Password strength: not set. Use 8 or more characters with an uppercase letter and a symbol."
        : "Password strength: " + label(met, v.length) + " (" + met + " of " + RULES.length + " requirements met)";
    }
    input.addEventListener("input", update);
    update();
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", setup);
  else setup();
})();
