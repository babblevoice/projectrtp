

#ifndef PROJECTRTPCODECX_H
#define PROJECTRTPCODECX_H

#include <iostream>
#include <cstdlib>

#include <ilbc.h>
#include <spandsp.h>

#include "projectrtppacket.h"
#include "projectrtppacket.h"
#include "projectrtprawsound.h"
#include "globals.h"
#include "projectrtpfirfilter.h"

/* ilbc v2/v3 differences */
#ifdef G_CONST /* ilbc v2 */
typedef WebRtc_Word16* ilbcencodedval;
typedef WebRtc_Word16* ilbcdecodedval;

typedef iLBC_decinst_t ilbcdecinst;
typedef iLBC_encinst_t ilbcencinst;
#else
typedef uint8_t* ilbcencodedval;
typedef int16_t* ilbcdecodedval;
typedef IlbcDecoderInstance ilbcdecinst;
typedef IlbcEncoderInstance ilbcencinst;
#endif

/*
Helper class for sound to manipulate sound - before or after a rtppacket is required.
*/

class codecx
{
  friend class rawsound;
public:
  codecx();
  ~codecx();

  codecx( const codecx& ) = delete;              // copy ctor
  codecx( codecx&& ) = delete;                   // move ctor
  codecx& operator=( const codecx& ) = delete;   // copy assignment
  codecx& operator=( codecx&& ) = delete;        // move assignment

  void reset( void );
  void restart( void );
  uint16_t power( void );

  bool hasdata() { return this->_hasdata; }
  rawsound& getref( int pt );

  friend codecx& operator << ( codecx&, rawsound& );
  friend codecx& operator << ( codecx&, rtppacket& );

  friend auto* operator << ( auto*, codecx& );
  friend auto& operator << ( auto&, codecx& );

  friend codecx& operator << ( codecx&, const char& );
  static const char next;

private:
  bool alaw2ulaw( void );
  bool ulaw2alaw( void );
  bool g711tol16( void );
  bool ilbctol16( void );
  bool g722tol16( void );
  bool l16topcma( void );
  bool l16topcmu( void );
  bool l16tog722( void );
  bool l16toilbc( void );

  bool l16lowtowideband( void);
  bool l16widetonarrowband( void );
  bool requirenarrowband( void );
  bool requirewideband( void );
  rawsound& requirel16( void );

  /* CODECs  */
  g722_encode_state_t *g722encoder;
  g722_decode_state_t *g722decoder;

  ilbcencinst *ilbcencoder;
  ilbcdecinst *ilbcdecoder;

  /* If we require downsampling */
  lowpass3_4k16k lpfilter;
  /* When we up sample we need to interpolate so need last sample */
  int16_t resamplelastsample;

  rawsound l168kref;
  rawsound l1616kref;
  rawsound pcmaref;
  rawsound pcmuref;
  rawsound g722ref;
  rawsound ilbcref;

  dcfilter dcpowerfilter;

  bool _hasdata;
  uint32_t inpkcount;
};

/* Functions */
void gen711convertdata( void );
void codectests( void );


/* Template functions */
/*
## rtppacket << codecx
Take the data out and transcode if necessary. Keep reference to any packet we have transcoded as we may need to use it again.

pk needs to impliment:
int getpayloadtype()
copy( *, size_t )
setpayloadlength( size_t )
setlength( size_t )
*/
auto& operator << ( auto& pk, codecx& c ) {
  (&pk) << c;
  return pk;
}

auto* operator << ( auto *pk, codecx& c ) {
  int outpayloadtype = pk->getpayloadtype();

  switch( outpayloadtype ) {
    case ILBCPAYLOADTYPE: {
      pk->setpayloadlength( ILBC20PAYLOADBYTES );

      if( !c.ilbcref.isdirty() ) {
        pk->copy( c.ilbcref.c_str(), c.ilbcref.size() * c.ilbcref.getbytespersample() );
        return pk;
      }
      c.ilbcref = rawsound( *pk, true );
      break;
    }
    case G722PAYLOADTYPE: {
      pk->setpayloadlength( G722PAYLOADBYTES );
      if( !c.g722ref.isdirty() ) {
        pk->copy( c.g722ref.c_str(), c.g722ref.size() * c.g722ref.getbytespersample() );
        return pk;
      }
      c.g722ref = rawsound( *pk, true );
      break;
    }
    case PCMAPAYLOADTYPE: {
      pk->setpayloadlength( G711PAYLOADBYTES );
      if( !c.pcmaref.isdirty() ) {
        pk->copy( c.pcmaref.c_str(), c.pcmaref.size() * c.pcmaref.getbytespersample() );
        return pk;
      }
      c.pcmaref = rawsound( *pk, true );
      break;
    }
    case PCMUPAYLOADTYPE: {
      pk->setpayloadlength( G711PAYLOADBYTES );
      if( !c.pcmuref.isdirty() ) {
        pk->copy( c.pcmuref.c_str(), c.pcmuref.size() * c.pcmuref.getbytespersample() );
        return pk;
      }
      c.pcmuref = rawsound( *pk, true );
      break;
    }
    case L168KPAYLOADTYPE: {
      pk->setpayloadlength( L16NARROWBANDBYTES );
      if( !c.l168kref.isdirty() ) {
        pk->copy( c.l168kref.c_str(), c.l168kref.size() * c.l168kref.getbytespersample() );
        return pk;
      }
      c.l168kref = rawsound( *pk, true );
      break;
    }

    case L1616KPAYLOADTYPE: {
      pk->setpayloadlength( L16WIDEBANDBYTES );
      if( !c.l1616kref.isdirty() ) {
        pk->copy( c.l1616kref.c_str(), c.l1616kref.size() * c.l1616kref.getbytespersample() );
        return pk;
      }
      c.l1616kref = rawsound( *pk, true );
      break;
    }
    default: {
      /* this should not happen */
      std::cerr << "Unknown CODEC when encoding: " << outpayloadtype << std::endl;
      return pk;
    }
  }

  /* encode */
  c.getref( outpayloadtype );

  return pk;
}

#ifdef NODE_MODULE
#include <node_api.h>
void initrtpcodecx( napi_env env, napi_value &result );
#endif



#endif /* PROJECTRTPCODECX_H */
