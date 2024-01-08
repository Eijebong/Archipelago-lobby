function onRepoTrClick(elm) {
    let child = elm.nextElementSibling;
    while (child) {
        let classes = child.className.split(' ')

        // If we don't have a git_ref class, we're on the next repo, break out
        if (!classes.includes('git_ref')) {
            break
        }

        if (classes.includes('hidden')) {
            classes = classes.filter(item => item != 'hidden')
        } else {
            classes.push('hidden')
        }

        child.className = classes.join(' ')
        child = child.nextElementSibling
    }
}

function triggerNewJob(org, repo, ref) {
    let url = new URL("/repos/trigger", document.location);
    const params = {org, repo, ref};
    url.search = new URLSearchParams(params).toString();

    fetch(url).then(async (res) => {
        if (!res.ok) {
            const text = await res.text()
            addMessage("error", text, 10000)
        } else {
            addMessage("success", "Succesfully started new job", 4000)
        }
    })
}

function addMessage(klass, text, delay){
    if(delay == undefined){
        delay = 5000;
    }
    messages_div = document.getElementById('messages');
    message = document.createElement('div');
    message_icon = document.createElement('i');
    message_span = document.createElement('span');

    if(klass == "error"){
        message_icon.className = "fa fa-exclamation-circle";
    }
    else if(klass == "warning"){
        message_icon.className = "fa fa-check-circle";
    }
    else if(klass == "info"){
        message_icon.className = "fa fa-info-circle";
    }
    else{
        message_icon.className = "fa fa-info-circle";
    }

    message_span.innerHTML = text;
    message.className = "message " + klass;
    messages_div.appendChild(message);
    message.appendChild(message_icon);
    message.appendChild(message_span);

    function remove(){
        // Avoid error when someone click on it before the setTimeout calls this function.
        if(this.parentNode != null){
            this.parentNode.removeChild(this);
        }
    }

    message.addEventListener('click', function(){
        remove.bind(this)();
    });
    if(delay){
        window.setTimeout(remove.bind(message), delay);
    }
}


