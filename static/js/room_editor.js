const tzOffset = (new Date()).getTimezoneOffset();
document.getElementById("tz_offset").value = tzOffset;


function dateToISOLikeButLocal(date) {
    const offsetMs = date.getTimezoneOffset() * 60 * 1000;
    const msLocal =  date.getTime() - offsetMs;
    const dateLocal = new Date(msLocal);
    const iso = dateLocal.toISOString();
    const isoLocal = iso.slice(0, 19);
    return isoLocal;
}

function updateFormset(formset) {
    const inputs = Array.from(formset.getElementsByTagName("input"));
    if (!inputs) {
        return;
    }

    const first_input = inputs[0];
    if (first_input.type != "checkbox") {
        return;
    }

    inputs.splice(0, 1);
    const display = first_input.checked ? "inline-block" : "none";
    for (const input of inputs) {
        if (input.type != "checkbox") {
            input.style.display = display;
        }

        for (const label of input.labels) {
            label.style.display = display;
        }
    }

    return first_input;
}

const formsets = document.getElementsByTagName("formset");
for (const formset of formsets) {
    const first_input = updateFormset(formset);
    if (first_input) {
        first_input.onchange = () => updateFormset(formset);
    }
}

const optionsSections = document.getElementsByClassName("options-tab");
const tabBar = document.getElementById("module-menu");
const tabs = tabBar.getElementsByTagName("a");

function switchToTab(tabId) {
    const selectedSection = document.getElementById("section-" + tabId);
    const selectedTab = document.getElementById(tabId);
    if (!selectedSection) {
        console.log("Section " + tabId + " doesn't exist, not switching")
        return
    }
    if (!selectedTab) {
        console.log("Tab " + tabId + " doesn't exist, not switching")
        return
    }

    for (const section of optionsSections) {
        section.style.display = 'none';
    }

    selectedSection.style.display = 'block';
    for (const tab of tabs) {
        tab.classList = ""
    }
    selectedTab.classList = "selected"
}

switchToTab(tabs[0].id)

for (const tab of tabs) {
    tab.addEventListener('click', () => {
        switchToTab(tab.id);
    })
}
