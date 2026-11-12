// Appearance before the first paint, so Sotto never flashes the wrong theme. Mirrors the
// attribute logic in src/theme/appearance.ts, which takes over once the app loads.
(function () {
  var a = {};
  try {
    a = JSON.parse(localStorage.getItem("sotto.appearance") || "{}") || {};
  } catch (e) {
    a = {};
  }
  var dark = a.mode === "dark" || (a.mode !== "light" && matchMedia("(prefers-color-scheme: dark)").matches);
  var theme = dark ? ({ dim: 1, dark: 1, midnight: 1 }[a.dark] ? a.dark : "dark") : a.light === "soft" ? "soft" : "light";
  var r = document.documentElement;
  r.setAttribute("data-theme", theme);
  r.setAttribute("data-scheme", dark ? "dark" : "light");
  r.setAttribute("data-accent", a.accent || "violet");
  r.setAttribute("data-bubbles", a.bubbles || "ink");
  r.setAttribute("data-wallpaper", a.wallpaper || "none");
  r.setAttribute("data-density", a.density || "comfortable");
  if (a.contrast) r.setAttribute("data-contrast", "more");
  if (a.reduceMotion) r.setAttribute("data-motion", "reduce");
  var bg = { light: "#f3f3f3", soft: "#e7e7ea", dim: "#16171a", dark: "#0b0b0c", midnight: "#000000" }[theme];
  var ink = {
    violet: ["#5b47c7", "#a896ff"],
    blue: ["#2160d8", "#7eaaff"],
    teal: ["#08777f", "#4fd1d9"],
    green: ["#1f7a3e", "#62d38c"],
    amber: ["#a3560b", "#f2b35b"],
    rose: ["#c0365e", "#ff8fae"],
    graphite: ["#3f3f46", "#d4d4d8"],
  }[a.accent] || ["#5b47c7", "#a896ff"];
  r.style.setProperty("--splash-bg", bg);
  r.style.setProperty("--splash-ink", ink[dark ? 1 : 0]);
  r.style.background = bg;
})();

// Pre-mount splash: the Sotto mark drawing itself. Hidden by the app once React has mounted.
(function () {
  var started = Date.now();
  window.__SPLASH__ = {
    hide: function () {
      var el = document.getElementById("splash");
      if (!el) return;
      var wait = Math.max(0, 350 - (Date.now() - started));
      setTimeout(function () {
        el.style.opacity = "0";
        setTimeout(function () {
          el.remove();
          var s = document.getElementById("splash-style");
          if (s) s.remove();
        }, 220);
      }, wait);
    },
  };
})();
