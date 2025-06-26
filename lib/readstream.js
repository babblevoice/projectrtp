const { default: axios } = require( "axios" )

module.exports.readstreamstart = async function( chan, msg ) {

  const rs = chan.readstream()
  let url = msg.url // We get this via babblertp?

  const startresp = await axios.post(
    "https://scribe-service-py-dev.bravesmoke-e232e428.uksouth.azurecontainerapps.io/api/audio-stream/start",
    { mimeType: "audio/webm;codecs=opus" },
    { headers: { "Content-Type": "application/json" } }
  )
  
  chan.readstream = rs
  const sessionid = startresp.data.data.session_id
  rs.sessionid = sessionid
  rs.msg = msg

  rs.on( "data", async ( chunk ) => {
  
    const base64Chunk = chunk.toString( "base64" )
  
    try {
      await axios.post(
        `https://scribe-service-py-dev.bravesmoke-e232e428.uksouth.azurecontainerapps.io/api/audio-stream/chunk/${sessionid}`,
        { chunk: base64Chunk },
        { headers: { "Content-Type": "application/json" } }
      )
    } catch ( err ) {
      console.log( err, "Error sending chunk" )
    }
  } )
}
  
module.exports.readstreamend = async function( chan ) {
  
  await axios.post(
      `https://scribe-service-py-dev.bravesmoke-e232e428.uksouth.azurecontainerapps.io/api/audio-stream/end/${chan.rs.sessionid}`,
      {
      mimeType: "audio/webm;codecs=opus",
      chunkDuration: 1000
      },
      { headers: { "Content-Type": "application/json" } }
    )
  const channelidentifiers = {
      "id": chan.rs.msg.id,
      "uuid": chan.rs.msg.uuid
      }
  chan.send( { ...{ "sessionid": chan.rs.sessionid }, ...channelidentifiers } )
}