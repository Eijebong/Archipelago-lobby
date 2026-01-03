function showYaml(roomId, yamlId, expandValidation) {
    const url = new URL("/api/room/" + roomId + "/info/" + yamlId, document.location)
    fetch(url)
        .then((response) => {
            if(!response.ok) {
                showError("Error while retrieving YAML: " + response.statusText);
                throw Error("Error while retrieving YAML: " + response.statusText);
            }
            return response.json()
        })
        .then((body) => {
            const title = body["player_name"] + " | " + body["game"]
            openYamlPopup(title, body["content"], roomId, yamlId, body["validation_status"], body["last_error"], expandValidation)
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

function openYamlPopup(title, yaml, roomId, yamlId, validationStatus, error, expandValidation) {
    const popup = document.createElement("dialog");
    popup.setAttribute("data-yaml-id", yamlId)
    popup.id = "yaml-content-popup"
    popup.classList = "popup";
    popup.onclick = (event) => { event.target == popup && popup.close(); return true; }

    const popupTitle = document.createElement("span");
    popupTitle.classList = "title"
    popupTitle.innerText = title;
    const yamlStatus = document.createElement("span");
    yamlStatus.classList = "validation-" + validationStatus.toLowerCase()
    yamlStatus.id = "yaml-status"
    yamlStatus.innerText = validationStatus

    popup.appendChild(popupTitle)
    popup.appendChild(yamlStatus)

    const errorInfo = document.createElement("pre")
    errorInfo.id = "yaml-error"
    if (error !== null) {
        errorInfo.innerText = error
    }
    popup.appendChild(errorInfo)

    yamlStatus.onclick = () => {
      const yamlError = document.getElementById("yaml-error")
      if (yamlError !== null) {
        yamlError.classList.toggle("visible-block")
      }
    }

    if (expandValidation) {
        errorInfo.classList.toggle("visible-block")
    }

    const popupContent = document.createElement("pre");
    popupContent.textContent = yaml;
    popupContent.id = "yaml-content";
    popupContent.classList = "language-yaml"
    popup.appendChild(popupContent);
    hljs.highlightElement(popupContent);

    const buttonContainer = document.createElement("div");
    buttonContainer.classList = "button-container";

    const downloadButton = document.createElement("button");
    downloadButton.innerText = "Download";
    downloadButton.classList = "button-emulator validation-button";
    downloadButton.onclick = () => { window.location.href = "/room/" + roomId + "/download/" + yamlId };

    const closeButton = document.createElement("button");
    closeButton.innerText = "Close";
    closeButton.classList = "button-emulator cancel-button";
    closeButton.onclick = () => { popup.close(); };
    popup.onclose = () => body.removeChild(popup);

    buttonContainer.appendChild(downloadButton);
    buttonContainer.appendChild(closeButton);
    popup.appendChild(buttonContainer);

    const body = document.getElementById("main");
    body.append(popup);

    popup.showModal()
}

function refreshPopupStatus(validationStatus, error) {
    const statusElmt = document.getElementById("yaml-status");
    statusElmt.classList = "validation-" + validationStatus.toLowerCase();
    statusElmt.innerText = validationStatus;

    const errorElmt = document.getElementById("yaml-error")
    errorElmt.innerText = error
}
