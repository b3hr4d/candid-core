/* Site behaviour: theme, mobile navigation, command-palette search, copy
 * buttons, table-of-contents scrollspy, and tab groups.
 *
 * A classic script, not a module, and it calls fetch() nowhere: the search
 * index arrives from assets/search-index.js as a global. That combination is
 * deliberate, because ES modules and fetch() are exactly what a file:// page
 * cannot use — a plain <script src> and a plain <link> can. Syntax
 * highlighting already happened at build time, so a reader with JavaScript
 * disabled still gets a complete, coloured page.
 */
(function () {
  "use strict";

  /* ------------------------------------------------------------- theme */

  var STORAGE_KEY = "candid-core-theme";

  function currentTheme() {
    return document.documentElement.getAttribute("data-theme") || "light";
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch (error) {
      /* Private browsing can refuse storage; the theme still applies. */
    }
    var button = document.getElementById("theme-toggle");
    if (button) {
      button.setAttribute(
        "aria-label",
        theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      );
    }
  }

  function initTheme() {
    var button = document.getElementById("theme-toggle");
    if (!button) return;
    applyTheme(currentTheme());
    button.addEventListener("click", function () {
      applyTheme(currentTheme() === "dark" ? "light" : "dark");
    });
  }

  /* --------------------------------------------------------- mobile nav */

  function initNav() {
    var toggle = document.getElementById("menu-toggle");
    var sidebar = document.querySelector(".sidebar");
    var backdrop = document.querySelector(".nav-backdrop");
    if (!toggle || !sidebar) return;

    function setOpen(open) {
      sidebar.setAttribute("data-open", open ? "true" : "false");
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
      if (backdrop) backdrop.hidden = !open;
    }

    toggle.addEventListener("click", function () {
      setOpen(sidebar.getAttribute("data-open") !== "true");
    });
    if (backdrop)
      backdrop.addEventListener("click", function () {
        setOpen(false);
      });
    sidebar.addEventListener("click", function (event) {
      if (event.target.closest("a")) setOpen(false);
    });
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape") setOpen(false);
    });
  }

  /* ------------------------------------------------------ copy buttons */

  function initCopy() {
    document.addEventListener("click", function (event) {
      var button = event.target.closest(".copy-button");
      if (!button) return;
      var block = button.closest(".code-block");
      var code = block && block.querySelector("pre > code");
      if (!code) return;

      var text = code.textContent;
      var done = function () {
        button.setAttribute("data-copied", "true");
        var label = button.querySelector(".copy-label");
        var previous = label ? label.textContent : "";
        if (label) label.textContent = "copied";
        setTimeout(function () {
          button.removeAttribute("data-copied");
          if (label) label.textContent = previous;
        }, 1400);
      };

      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, fallback);
      } else {
        fallback();
      }

      function fallback() {
        var area = document.createElement("textarea");
        area.value = text;
        area.setAttribute("readonly", "");
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        try {
          document.execCommand("copy");
          done();
        } catch (error) {
          /* Nothing sensible to do; the code is still selectable by hand. */
        }
        document.body.removeChild(area);
      }
    });
  }

  /* ------------------------------------------------------------- tabs  */

  function initTabs() {
    document.querySelectorAll(".tabs").forEach(function (group) {
      var buttons = Array.prototype.slice.call(group.querySelectorAll(".tab-list button"));
      var panels = Array.prototype.slice.call(group.querySelectorAll(".tab-panel"));
      if (buttons.length === 0) return;

      function select(index) {
        buttons.forEach(function (button, i) {
          button.setAttribute("aria-selected", i === index ? "true" : "false");
          button.tabIndex = i === index ? 0 : -1;
        });
        panels.forEach(function (panel, i) {
          panel.hidden = i !== index;
        });
      }

      buttons.forEach(function (button, index) {
        button.addEventListener("click", function () {
          select(index);
        });
        button.addEventListener("keydown", function (event) {
          var next =
            event.key === "ArrowRight" ? index + 1 : event.key === "ArrowLeft" ? index - 1 : -1;
          if (next < 0 || next >= buttons.length) return;
          event.preventDefault();
          select(next);
          buttons[next].focus();
        });
      });

      select(0);
    });
  }

  /* ---------------------------------------------------------- scrollspy */

  function initScrollspy() {
    var links = Array.prototype.slice.call(document.querySelectorAll(".toc a[href^='#']"));
    if (links.length === 0 || !window.IntersectionObserver) return;

    var byId = {};
    var headings = [];
    links.forEach(function (link) {
      var id = decodeURIComponent(link.getAttribute("href").slice(1));
      var heading = document.getElementById(id);
      if (!heading) return;
      byId[id] = link;
      headings.push(heading);
    });

    var visible = new Set();

    function refresh() {
      var best = null;
      headings.forEach(function (heading) {
        if (visible.has(heading.id)) {
          if (!best || heading.compareDocumentPosition(best) & Node.DOCUMENT_POSITION_PRECEDING)
            return;
          best = heading;
        }
      });
      if (!best) {
        // Nothing intersecting: fall back to the last heading scrolled past.
        for (var i = headings.length - 1; i >= 0; i -= 1) {
          if (headings[i].getBoundingClientRect().top < 120) {
            best = headings[i];
            break;
          }
        }
      }
      links.forEach(function (link) {
        link.classList.remove("active");
      });
      if (best && byId[best.id]) byId[best.id].classList.add("active");
    }

    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (entry.isIntersecting) visible.add(entry.target.id);
          else visible.delete(entry.target.id);
        });
        refresh();
      },
      { rootMargin: "-80px 0px -70% 0px", threshold: 0 },
    );

    headings.forEach(function (heading) {
      observer.observe(heading);
    });
    window.addEventListener("scroll", refresh, { passive: true });
    refresh();
  }

  /* ------------------------------------------------------------ search */

  function initSearch() {
    var trigger = document.getElementById("search-trigger");
    var dialog = document.getElementById("search-dialog");
    var input = document.getElementById("search-input");
    var results = document.getElementById("search-results");
    if (!trigger || !dialog || !input || !results) return;

    var index = window.CANDID_CORE_SEARCH_INDEX || [];
    var base = document.documentElement.getAttribute("data-base") || "";
    var selected = 0;
    var current = [];

    function open() {
      if (typeof dialog.showModal === "function") dialog.showModal();
      else dialog.setAttribute("open", "");
      input.value = "";
      render([]);
      input.focus();
    }

    function close() {
      if (typeof dialog.close === "function") dialog.close();
      else dialog.removeAttribute("open");
    }

    function score(entry, terms) {
      var total = 0;
      for (var i = 0; i < terms.length; i += 1) {
        var term = terms[i];
        var inTitle = entry.title.toLowerCase().indexOf(term);
        var inHeading = entry.headings.toLowerCase().indexOf(term);
        var inBody = entry.body.indexOf(term);
        if (inTitle === -1 && inHeading === -1 && inBody === -1) return 0;
        if (inTitle === 0) total += 60;
        else if (inTitle > -1) total += 34;
        if (inHeading > -1) total += 14;
        if (inBody > -1) total += 5;
      }
      return total;
    }

    function snippet(entry, term) {
      var at = entry.body.indexOf(term);
      if (at === -1) return entry.lead || entry.body.slice(0, 140);
      var start = Math.max(0, at - 50);
      return (start > 0 ? "…" : "") + entry.body.slice(start, start + 150).trim() + "…";
    }

    function render(list) {
      current = list;
      selected = 0;
      results.innerHTML = "";
      if (list.length === 0) {
        var message = document.createElement("li");
        message.className = "search-empty";
        message.textContent = input.value.trim()
          ? "No matches."
          : "Search titles, headings and page text.";
        results.appendChild(message);
        return;
      }
      list.forEach(function (entry, i) {
        var item = document.createElement("li");
        item.setAttribute("aria-selected", i === 0 ? "true" : "false");
        var link = document.createElement("a");
        link.href = base + entry.url;
        link.innerHTML =
          '<span class="search-title"></span><span class="search-crumb"></span><span class="search-snippet"></span>';
        link.querySelector(".search-title").textContent = entry.title;
        link.querySelector(".search-crumb").textContent = entry.section;
        link.querySelector(".search-snippet").textContent = entry.snippet;
        item.appendChild(link);
        results.appendChild(item);
      });
    }

    function search() {
      var query = input.value.trim().toLowerCase();
      if (!query) return render([]);
      var terms = query.split(/\s+/);
      var scored = [];
      index.forEach(function (entry) {
        var value = score(entry, terms);
        if (value > 0) {
          scored.push({
            title: entry.title,
            section: entry.section,
            url: entry.url,
            snippet: snippet(entry, terms[0]),
            score: value,
          });
        }
      });
      scored.sort(function (a, b) {
        return b.score - a.score;
      });
      render(scored.slice(0, 12));
    }

    function move(delta) {
      if (current.length === 0) return;
      var items = results.querySelectorAll("li");
      items[selected] && items[selected].setAttribute("aria-selected", "false");
      selected = (selected + delta + current.length) % current.length;
      var item = items[selected];
      if (item) {
        item.setAttribute("aria-selected", "true");
        item.scrollIntoView({ block: "nearest" });
      }
    }

    trigger.addEventListener("click", open);
    input.addEventListener("input", search);

    dialog.addEventListener("keydown", function (event) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        move(1);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        move(-1);
      } else if (event.key === "Enter") {
        var items = results.querySelectorAll("li a");
        if (items[selected]) {
          event.preventDefault();
          window.location.href = items[selected].href;
        }
      }
    });

    dialog.addEventListener("click", function (event) {
      if (event.target === dialog) close();
    });

    document.addEventListener("keydown", function (event) {
      var isShortcut = (event.key === "k" && (event.metaKey || event.ctrlKey)) || event.key === "/";
      if (!isShortcut) return;
      var tag = document.activeElement && document.activeElement.tagName;
      if (event.key === "/" && (tag === "INPUT" || tag === "TEXTAREA")) return;
      event.preventDefault();
      open();
    });
  }

  function ready(fn) {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", fn);
    else fn();
  }

  ready(function () {
    initTheme();
    initNav();
    initCopy();
    initTabs();
    initScrollspy();
    initSearch();
  });
})();
