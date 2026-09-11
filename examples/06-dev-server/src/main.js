// 06 - Dev Server
// Browser-side code served by `3va dev` with HMR.

const app = document.getElementById("app");

function render() {
  const greeting = "Hello from 3va!";
  app.innerHTML = `
    <p>
      <strong>${greeting}</strong><br>
      Click the button to add a task:
    </p>
    <button id="add-todo">Add task</button>
    <ul id="todos"></ul>
  `;

  const button = document.getElementById("add-todo");
  const list = document.getElementById("todos");
  let count = 0;

  button.addEventListener("click", () => {
    count += 1;
    const li = document.createElement("li");
    li.textContent = `Task ${count} — created at ${new Date().toLocaleTimeString()}`;
    list.appendChild(li);
  });
}

render();