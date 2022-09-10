/* remote mode */
const projectrtp = require( "@babblevoice/projectrtp" ).projectrtp
async function connect() {
    projectrtp.proxy.listen()
    console.log("Server listening")
    await new Promise( ( r ) => { setTimeout( () => r(),  5000  ) } )

    let channela = await projectrtp.openchannel({}, ( d ) => {
      if( "mix" === d.action ) {
        console.log("mixed channel a")
      }
    } )
    let channelb = await projectrtp.openchannel({}, ( d ) => {
        if( "mix" === d.action ) {
          console.log("mixed channel b")
        }
    } )

    let clienta = await projectrtp.openchannel({ "remote": { "address": "localhost", "port": channela.local.port, "codec": 0 } })
    let clientb = await projectrtp.openchannel({ "remote": { "address": "localhost", "port": channelb.local.port, "codec": 0 } })

    await channela.mix(channelb)
    let channelc = await projectrtp.openchannel()
    await channela.mix(channelc)

    console.log("Channels mixed")
    await new Promise( ( r ) => { setTimeout( () => r(),  5000  ) } )
    clienta.play( { "loop": true, "files": [ { "wav": "/tmp/uksounds.wav" } ] } ) 
    clientb.echo()
    channela.close()

    await new Promise( ( r ) => { setTimeout( () => r(),  10000  ) } )
}

connect()