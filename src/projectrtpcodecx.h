

#ifndef PROJECTRTPCODECX_H
#define PROJECTRTPCODECX_H


#include <ilbc.h>
#include <spandsp.h>

#include "projectrtppacket.h"
#include "projectrtppacket.h"
#include "globals.h"
#include "firfilter.h"


/*
Helper class for sound to manipulate sound - before or after a rtppacket is required.
*/
class codecx;
class rawsound
{
public:
  rawsound();
  rawsound( uint8_t *ptr, std::size_t samples, int format, uint16_t samplerate );
  rawsound( rtppacket& pk );
  rawsound( rawsound & );
  ~rawsound();

  uint8_t *c_str( void ){ return this->data; };
  size_t size( void ){ return this->samples; };
  void size( size_t samplecount ){ this->samples = samplecount; };
  inline int getformat( void ){ return this->format; }; /* aka payload type */
  uint16_t getsamplerate( void ) { return this->samplerate; };
  void malloc( size_t samplecount, size_t bytespersample, int format );
  void zero( void );

  /* needed for << operato on codecx */
  inline int getpayloadtype( void ) { return this->format; }
  inline void setpayloadlength( size_t length ) { this->samples = length; };
  inline void setlength( size_t length ) { this->samples = length; };
  void copy( uint8_t *src, size_t len );
  void copy( rawsound &other );

  rawsound& operator+=( codecx& rhs );
  rawsound& operator-=( codecx& rhs );

private:
  void frompt( int payloadtype );

  /* ptr to our buffer */
  uint8_t *data;

  /* sample count of buffer in bytes */
  size_t samples;

  /* The amount we requested from the system malloc */
  size_t allocatedlength;

  size_t bytespersample;

  /* see globals.h */
  int format;
  uint16_t samplerate;
};

class codecx
{
  friend class rawsound;
public:
  codecx();
  ~codecx();

  void reset( void );
  void restart( void );
  uint16_t power( void );

  friend codecx& operator << ( codecx&, rawsound& );
  friend codecx& operator << ( codecx&, rtppacket& );

  friend auto* operator << ( auto*, codecx& );
  friend auto& operator << ( auto&, codecx& );

  friend codecx& operator << ( codecx&, const char& );
  static const char next;

private:
  void alaw2ulaw( void );
  void ulaw2alaw( void );
  bool g711tol16( void );
  bool ilbctol16( void );
  bool g722tol16( void );
  void l16topcma( void );
  void l16topcmu( void );
  void l16tog722( void );
  void l16toilbc( void );

  void l16lowtowideband( void);
  void l16widetonarrowband( void );
  void requirenarrowband( void );
  void requirewideband( void );
  rawsound& requirel16( void );

  /* CODECs  */
  g722_encode_state_t *g722encoder;
  g722_decode_state_t *g722decoder;

  iLBC_encinst_t *ilbcencoder;
  iLBC_decinst_t *ilbcdecoder;

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
auto& operator << ( auto& pk, codecx& c )
{
  (&pk) << c;
  return pk;
}

auto* operator << ( auto *pk, codecx& c )
{
  int outpayloadtype = pk->getpayloadtype();

  /* If we have already have or converted this packet... */
  if( PCMAPAYLOADTYPE == outpayloadtype &&  0 != c.pcmaref.size() )
  {
    pk->copy( c.pcmaref.c_str(), c.pcmaref.size() );
    return pk;
  }
  else if( PCMUPAYLOADTYPE == outpayloadtype && 0 != c.pcmuref.size() )
  {
    pk->copy( c.pcmuref.c_str(), c.pcmuref.size() );
    return pk;
  }
  else if( ILBCPAYLOADTYPE == outpayloadtype && 0 != c.ilbcref.size() )
  {
    pk->copy( c.ilbcref.c_str(), c.ilbcref.size() );
    return pk;
  }
  else if( G722PAYLOADTYPE == outpayloadtype && 0 != c.g722ref.size() )
  {
    pk->copy( c.g722ref.c_str(), c.g722ref.size() );
    return pk;
  }

  /* If we get here we may have L16 but at the wrong sample rate so check and resample - then convert */
  /* narrowband targets */
  switch( outpayloadtype )
  {
    case ILBCPAYLOADTYPE:
    {
      c.requirenarrowband();
      c.ilbcref = rawsound( *pk );
      c.l16toilbc();
      pk->setpayloadlength( c.ilbcref.size() );
      break;
    }
    case G722PAYLOADTYPE:
    {
      c.requirewideband();
      c.g722ref = rawsound( *pk );
      c.l16tog722();
      pk->setpayloadlength( c.g722ref.size() );
      break;
    }
    case PCMAPAYLOADTYPE:
    {
      if( c.pcmuref.size() > 0 )
      {
        c.pcmaref = rawsound( *pk );
        c.alaw2ulaw();
      }
      else
      {
        c.requirenarrowband();
        c.pcmaref = rawsound( *pk );

        c.l16topcma();
      }

      pk->setpayloadlength( c.pcmaref.size() );

      break;
    }
    case PCMUPAYLOADTYPE:
    {
      if( c.pcmaref.size() > 0 )
      {
        c.pcmuref = rawsound( *pk );
        c.ulaw2alaw();
      }
      else
      {
        c.requirenarrowband();

        c.pcmuref = rawsound( *pk );
        c.l16topcmu();
      }

      pk->setpayloadlength( c.pcmuref.size() );
      break;
    }
    default:
    {
      pk->setlength( 0 );
    }
  }

  return pk;
}


#endif /* PROJECTRTPCODECX_H */
