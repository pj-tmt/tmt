
const root = document.querySelector("#blog-article .usage-timeline");
if (root) {
  const detail = root.querySelector(".tl-detail");
  const events = [...root.querySelectorAll(".tl-list button")];
  const filters = [...root.querySelectorAll(".tl-filter button")];
  const show = (button) => {
    events.forEach((b) => b.setAttribute("aria-expanded", String(b === button)));
    const strong = document.createElement("strong");
    strong.textContent = button.querySelector(".tl-date").textContent + " · " + button.querySelector("span").textContent;
    detail.replaceChildren(strong, " " + button.querySelector("small").textContent);
  };
  events.forEach((b) => b.addEventListener("click", () => show(b)));
  filters.forEach((f) => f.addEventListener("click", () => {
    filters.forEach((g) => g.classList.toggle("on", g === f));
    const who = f.value;
    events.forEach((b) => {
      const match = who === "all" || b.classList.contains("tl-" + who) || (b.classList.contains("tl-both") && who !== "us");
      b.parentElement.hidden = !match;
    });
  }));
}
