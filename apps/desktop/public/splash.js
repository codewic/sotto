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
