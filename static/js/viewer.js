function showYaml(roomId, yamlId, yamlName, yamlGame) {
    const url = new URL("/room/" + roomId + "/download/" + yamlId, document.location)
    fetch(url)
        .then((response) => {
            if(!response.ok) {
                showError("Error while retrieving YAML:" + response.statusText);
            }
            return response.text()
        })
        .then((body) => {
            const title = yamlName + " | " + yamlGame;
            openYamlPopup(title, body, roomId, yamlId)
        })

    return false;
}

function showError(msg) {
    const messages = document.getElementById("messages");
    const error = document.createElement('p');
    error.classList = "message error";
    error.innerText = msg;
    messages.append(error);

    setTimeout(() => { error.remove() }, 5000);
}

function openYamlPopup(title, yaml, roomId, yamlId) {
    const popup = document.createElement("dialog");
    popup.classList = "popup";
    popup.onclick = (event) => { event.target == popup && popup.close(); return true; }

    const popupTitle = document.createElement("span");
    popupTitle.classList = "title"
    popupTitle.innerText = title;
    popup.appendChild(popupTitle)

    const popupContent = document.createElement("pre");
    popupContent.textContent = yaml;
    popupContent.classList = "language-yaml"
    popup.appendChild(popupContent);
    hljs.highlightElement(popupContent);

    const buttonContainer = document.createElement("div");
    buttonContainer.classList = "button-container";

    const downloadButton = document.createElement("a");
    downloadButton.innerText = "Download";
    downloadButton.classList = "validation-button";
    downloadButton.href = "/room/" + roomId + "/download/" + yamlId;

    const closeButton = document.createElement("button");
    closeButton.innerText = "Close";
    closeButton.classList = "cancel-button";
    closeButton.onclick = () => { popup.close(); };
    popup.onclose = () => body.removeChild(popup);

    buttonContainer.appendChild(downloadButton);
    buttonContainer.appendChild(closeButton);
    popup.appendChild(buttonContainer);

    const body = document.getElementById("main");
    body.append(popup);

    popup.showModal()
}
