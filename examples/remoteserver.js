/* remote mode */
const projectrtp = require( "@babblevoice/projectrtp" ).projectrtp
async function connect() {
    projectrtp.proxy.listen()
    console.log("Server listening")
    await new Promise( ( r ) => { setTimeout( () => r(),  10000  ) } )
    projectrtp.openchannel()
    console.log("Channel opened")
    await new Promise( ( r ) => { setTimeout( () => r(),  10000  ) } )
    projectrtp.openchannel()
    console.log("Channel opened")
}

connect()