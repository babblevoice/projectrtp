/* remote mode */
const projectrtp = require( "@babblevoice/projectrtp" ).projectrtp
async function connect() {
    projectrtp.proxy.listen()
    console.log("Server listening")
    await new Promise( ( r ) => { setTimeout( () => r(),  5000  ) } )

    let clienta = await projectrtp.openchannel()
    let clientb = await projectrtp.openchannel({ "remote": { "address": "localhost", "port": clienta.remoteport, "codec": 0 } })
    clienta.remote({ "address": "localhost", "port": clientb.remoteport, "codec": 0 })

    let channela = await projectrtp.openchannel()
    let channelb = await projectrtp.openchannel()
    channela.mix(channelb)

    console.log("Channels mixed")
    clienta.play( { "loop": true, "files": [ { "wav": "/tmp/uksounds.wav" } ] } ) 
    clientb.echo()
    await new Promise( ( r ) => { setTimeout( () => r(),  10000  ) } )
}

connect()