
#include <iostream>

#include "projectrtpcodecx.h"

const char codecx::next = 0;

/*!md
# Project CODECs
This file is responsible for converting different types of rtppackets into different CODECs. It is fixed for now - for efficiency and simplicity. Perhaps in the future we could support more and pluggable CODECs.
*/

/*!md
Pre generate all 711 data.
We can speed up 711 conversion by way of pre calculating values. 128K(ish) of data is not too much to worry about!
*/

static uint8_t _l16topcmu[ 65536 ];
static uint8_t _l16topcma[ 65536 ];
static int16_t _pcmatol16[ 256 ];
static int16_t _pcmutol16[ 256 ];

/* Copied from the CCITT G.711 specification  - taken from spandsp*/
static const uint8_t ulaw_to_alaw_table[256] =
{
     42,  43,  40,  41,  46,  47,  44,  45,  34,  35,  32,  33,  38,  39,  36,  37,
     58,  59,  56,  57,  62,  63,  60,  61,  50,  51,  48,  49,  54,  55,  52,  53,
     10,  11,   8,   9,  14,  15,  12,  13,   2,   3,   0,   1,   6,   7,   4,  26,
     27,  24,  25,  30,  31,  28,  29,  18,  19,  16,  17,  22,  23,  20,  21, 106,
    104, 105, 110, 111, 108, 109,  98,  99,  96,  97, 102, 103, 100, 101, 122, 120,
    126, 127, 124, 125, 114, 115, 112, 113, 118, 119, 116, 117,  75,  73,  79,  77,
     66,  67,  64,  65,  70,  71,  68,  69,  90,  91,  88,  89,  94,  95,  92,  93,
     82,  82,  83,  83,  80,  80,  81,  81,  86,  86,  87,  87,  84,  84,  85,  85,
    170, 171, 168, 169, 174, 175, 172, 173, 162, 163, 160, 161, 166, 167, 164, 165,
    186, 187, 184, 185, 190, 191, 188, 189, 178, 179, 176, 177, 182, 183, 180, 181,
    138, 139, 136, 137, 142, 143, 140, 141, 130, 131, 128, 129, 134, 135, 132, 154,
    155, 152, 153, 158, 159, 156, 157, 146, 147, 144, 145, 150, 151, 148, 149, 234,
    232, 233, 238, 239, 236, 237, 226, 227, 224, 225, 230, 231, 228, 229, 250, 248,
    254, 255, 252, 253, 242, 243, 240, 241, 246, 247, 244, 245, 203, 201, 207, 205,
    194, 195, 192, 193, 198, 199, 196, 197, 218, 219, 216, 217, 222, 223, 220, 221,
    210, 210, 211, 211, 208, 208, 209, 209, 214, 214, 215, 215, 212, 212, 213, 213
};

/* These transcoding tables are copied from the CCITT G.711 specification. To achieve
   optimal results, do not change them. */

static const uint8_t alaw_to_ulaw_table[256] =
{
     42,  43,  40,  41,  46,  47,  44,  45,  34,  35,  32,  33,  38,  39,  36,  37,
     57,  58,  55,  56,  61,  62,  59,  60,  49,  50,  47,  48,  53,  54,  51,  52,
     10,  11,   8,   9,  14,  15,  12,  13,   2,   3,   0,   1,   6,   7,   4,   5,
     26,  27,  24,  25,  30,  31,  28,  29,  18,  19,  16,  17,  22,  23,  20,  21,
     98,  99,  96,  97, 102, 103, 100, 101,  93,  93,  92,  92,  95,  95,  94,  94,
    116, 118, 112, 114, 124, 126, 120, 122, 106, 107, 104, 105, 110, 111, 108, 109,
     72,  73,  70,  71,  76,  77,  74,  75,  64,  65,  63,  63,  68,  69,  66,  67,
     86,  87,  84,  85,  90,  91,  88,  89,  79,  79,  78,  78,  82,  83,  80,  81,
    170, 171, 168, 169, 174, 175, 172, 173, 162, 163, 160, 161, 166, 167, 164, 165,
    185, 186, 183, 184, 189, 190, 187, 188, 177, 178, 175, 176, 181, 182, 179, 180,
    138, 139, 136, 137, 142, 143, 140, 141, 130, 131, 128, 129, 134, 135, 132, 133,
    154, 155, 152, 153, 158, 159, 156, 157, 146, 147, 144, 145, 150, 151, 148, 149,
    226, 227, 224, 225, 230, 231, 228, 229, 221, 221, 220, 220, 223, 223, 222, 222,
    244, 246, 240, 242, 252, 254, 248, 250, 234, 235, 232, 233, 238, 239, 236, 237,
    200, 201, 198, 199, 204, 205, 202, 203, 192, 193, 191, 191, 196, 197, 194, 195,
    214, 215, 212, 213, 218, 219, 216, 217, 207, 207, 206, 206, 210, 211, 208, 209
};

void gen711convertdata( void )
{
  std::cout << "Pre generating G711 tables";
  for( int32_t i = 0; i != 65535; i++ )
  {
    int16_t l16val = i - 32768;
    _l16topcmu[ i ] = linear_to_ulaw( l16val );
    _l16topcma[ i ] = linear_to_alaw( l16val );
  }

  for( uint8_t i = 0; i != 255; i++ )
  {
    _pcmatol16[ i ] = alaw_to_linear( i );
    _pcmutol16[ i ] = ulaw_to_linear( i );
  }

  std::cout << " - completed." << std::endl;
}

codecx::codecx() :
  g722encoder( nullptr ),
  g722decoder( nullptr ),
  ilbcencoder( nullptr ),
  ilbcdecoder( nullptr ),
  resamplelastsample( 0 )
{

}

codecx::~codecx()
{
  this->reset();
}

/*!md
## reset
Full reset - clear out all CODECs
*/
void codecx::reset()
{
  if( nullptr != this->g722decoder )
  {
    g722_decode_free( this->g722decoder );
    this->g722decoder = nullptr;
  }
  if( nullptr != this->g722encoder )
  {
    g722_encode_free( this->g722encoder );
    this->g722encoder = nullptr;
  }

  if( nullptr != this->ilbcdecoder )
  {
    WebRtcIlbcfix_DecoderFree( this->ilbcdecoder );
    this->ilbcdecoder = nullptr;
  }
  if( nullptr != this->ilbcencoder )
  {
    WebRtcIlbcfix_EncoderFree( this->ilbcencoder );
    this->ilbcencoder = nullptr;
  }

  this->restart();

}

/*!md
## restart
Do enough to manage missing packets.
*/
void codecx::restart( void )
{
  this->lpfilter.reset();
  this->resamplelastsample = 0;
}

/*!md
## ulaw2alaw, alaw2ulaw

From whichever PCM encoding (u or a) encode to the other without having to do intermediate l16.

TODO - check that using a lookup table instead of a funcion call uses SSE functions in the CPU.
*/
void codecx::ulaw2alaw( void )
{
  uint8_t *inbufptr, *outbufptr;
  size_t insize;

  insize = this->pcmaref.size();
  inbufptr = this->pcmaref.c_str();
  outbufptr = this->pcmuref.c_str();
  this->pcmuref.size( insize );

  if( nullptr == outbufptr || nullptr == inbufptr )
  {
    std::cerr << "PCMA NULLPTR shouldn't happen (" << (void*)outbufptr << ", " << (void*)inbufptr << ")" << std::endl;
    return;
  }

  for( size_t i = 0; i < insize; i++ )
  {
    *outbufptr++ = alaw_to_ulaw_table[ *inbufptr++ ];
  }
}

void codecx::alaw2ulaw( void )
{
  uint8_t *inbufptr, *outbufptr;
  size_t insize;

  insize = this->pcmuref.size();
  inbufptr = this->pcmuref.c_str();
  outbufptr = this->pcmaref.c_str();
  this->pcmaref.size( insize );

  if( nullptr == outbufptr || nullptr == inbufptr )
  {
    std::cerr << "PCMU NULLPTR shouldn't happen(" << (void*)outbufptr << ", " << (void*)inbufptr << ")" << std::endl;
    return;
  }

  for( size_t i = 0; i < insize; i++ )
  {
    *outbufptr++ = ulaw_to_alaw_table[ *inbufptr++ ];
  }
}

/*!md
## g711tol16
As it says.
*/
bool codecx::g711tol16( void )
{
  uint8_t *in;
  int16_t *convert;
  size_t insize;

  if( this->pcmaref.size() > 0 )
  {
    in = this->pcmaref.c_str();
    convert = _pcmatol16;
    insize = this->pcmaref.size();
  }
  else if ( this->pcmuref.size() > 0 )
  {
    in = this->pcmuref.c_str();
    convert = _pcmutol16;
    insize = this->pcmuref.size();
  }
  else
  {
    return false;
  }

  this->l168kref.malloc( insize * 2 );

  int16_t *out = ( int16_t * ) this->l168kref.c_str();

  for( size_t i = 0; i < insize; i++ )
  {
    *out = convert[ *in ];
    in++;
    out++;
  }

  return true;
}


/*!md
## l16topcma
*/
void codecx::l16topcma( void )
{
  uint8_t *out = this->pcmaref.c_str();

  int16_t *in;
  size_t l168klength = this->l168kref.size() / 2;
  in = ( int16_t * ) this->l168kref.c_str();


  for( size_t i = 0; i < l168klength; i++ )
  {
    *out = _l16topcma[ ( *in ) + 32768 ];

    in++;
    out++;
  }
}

/*!md
## l16topcmu
*/
void codecx::l16topcmu( void )
{
  uint8_t *out = this->pcmuref.c_str();;

  int16_t *in;
  size_t l168klength = this->l168kref.size() / 2;
  in = ( int16_t * ) this->l168kref.c_str();

  for( size_t i = 0; i < l168klength; i++ )
  {
    *out = _l16topcmu[ ( *in ) + 32768 ];

    in++;
    out++;
  }
}

/*!md
## ilbctol16
As it says.
*/
bool codecx::ilbctol16( void )
{
  if( 0 == this->ilbcref.size() ) return false;

  /* roughly compression size with some leg room. */
  this->l168kref.malloc( this->ilbcref.size() * 5 );

  WebRtc_Word16 speechType;

  if( nullptr == this->ilbcdecoder )
  {
    /* Only support 20mS ATM to make the mixing simpler */
    WebRtcIlbcfix_DecoderCreate( &this->ilbcdecoder );
    WebRtcIlbcfix_DecoderInit( this->ilbcdecoder, 20 );
  }

  WebRtc_Word16 l168klength = WebRtcIlbcfix_Decode( this->ilbcdecoder,
                        ( WebRtc_Word16* ) this->ilbcref.c_str(),
                        this->ilbcref.size(),
                        ( WebRtc_Word16 * )this->l168kref.c_str(),
                        &speechType
                      );

  if( -1 == l168klength )
  {
    this->l168kref.size( 0 );
    return false;
  }

  this->l168kref.size( l168klength );
  return true;
}

/*!md
## l16tog722
As it says.
*/
void codecx::l16tog722( void )
{
  if( 0 == this->l1616kref.size() )
  {
    return;
  }

  if( nullptr == this->g722encoder )
  {
    this->g722encoder = g722_encode_init( NULL, 64000, G722_PACKED );
  }

  int len = g722_encode( this->g722encoder, this->g722ref.c_str(), ( int16_t * ) this->l1616kref.c_str(), this->g722ref.size() * 2 );

  if( len > 0 )
  {
    this->g722ref.size( len );
  }
  else
  {
    this->g722ref.size( 0 );
  }
}

/*!md
## l16toilbc
As it says.
I think we can always send as 20mS.
According to RFC 3952 you determin how many frames are in the packet by the frame length. This is also true (although not explicit) that we can send as 20mS if our source is that but also 30mS if that is the case.

As we only support G722, G711 and iLBC (20/30) then we should be able simply encode and send as the matched size.
*/
void codecx::l16toilbc( void )
{
  if( 0 == this->l168kref.size() )
  {
    return;
  }

  if( nullptr == this->ilbcencoder )
  {
    /* Only support 20mS ATM to make the mixing simpler */
    WebRtcIlbcfix_EncoderCreate( &this->ilbcencoder );
    WebRtcIlbcfix_EncoderInit( this->ilbcencoder, 20 );
  }

  WebRtc_Word16 len = WebRtcIlbcfix_Encode( this->ilbcencoder,
                            ( WebRtc_Word16 * ) this->l168kref.c_str(),
                            this->l168kref.size() / 2,
                            ( WebRtc_Word16* ) this->ilbcref.c_str()
                          );
  if ( len > 0 )
  {
    this->ilbcref.size( len );
  }
  else
  {
    this->ilbcref.size( 0 );
  }
}


/*!md
## g722tol16
As it says.
*/
bool codecx::g722tol16( void )
{
  if( 0 == this->g722ref.size() )
  {
    return false;
  }

  this->l168kref.malloc( this->g722ref.size() * 2 );

  if( nullptr == this->g722decoder )
  {
    this->g722decoder = g722_decode_init( NULL, 64000, G722_PACKED );
  }

  size_t l1616klength = g722_decode( g722decoder, ( int16_t * ) this->l1616kref.c_str(), this->g722ref.c_str(), this->g722ref.size() );
  this->l1616kref.size( l1616klength );

  return true;
}

/*!md
## l16lowtowideband
Upsample from narrow to wideband. Take each point and interpolate between them. We require the final sample from the last packet to continue the interpolating.
*/
void codecx::l16lowtowideband( void )
{
  size_t l168klength = this->l168kref.size();
  if( 0 == l168klength )
  {
    return;
  }

  this->l1616kref.malloc( l168klength * 2 );

  int16_t *in = ( int16_t * ) this->l168kref.c_str();
  int16_t *out = ( int16_t * ) this->l1616kref.c_str();

  for( size_t i = 0; i < l168klength/2; i++ )
  {
    *out = ( ( *in - this->resamplelastsample ) / 2 ) + this->resamplelastsample;
    this->resamplelastsample = *in;
    out++;

    *out = *in;

    out++;
    in++;
  }
}

/*!md
## requirenarrowband
Search for the relevent data and convert as necessary.
*/
void codecx::requirewideband( void )
{
  if( 0 != this->l1616kref.size() ) return;
  if( this->g722tol16() ) return;
  if( !this->g711tol16() )
  {
    if( this->ilbctol16() )
    {
      return;
    }
  }

  this->l16lowtowideband();
}

/*!md
##  l16widetolowband
Downsample our L16 wideband samples to 8K. Pass through filter then grab every other sample.
*/
void codecx::l16widetonarrowband( void )
{
  size_t l1616klength = this->l1616kref.size();
  if( 0 == l1616klength )
  {
    return;
  }

  /* if odd need ceil instead of floor (the + 1 bit) */
  size_t l168klength = ( l1616klength + 1 ) / 2;
  this->l168kref.malloc( l168klength );

  int16_t *out = ( int16_t * ) this->l168kref.c_str();
  int16_t *in = ( int16_t * ) this->l1616kref.c_str();

  int16_t filteredval;
  for( size_t i = 0; i < l1616klength; i++ )
  {
    filteredval = lpfilter.execute( *in );
    in++;

    if( 0x01 == ( i & 0x01 ) )
    {
      *out = filteredval;
      out++;
    }
  }
}

/*!md
## requirenarrowband
Search for the relevent data and convert as necessary.
*/
void codecx::requirenarrowband( void )
{
  if( 0 != this->l168kref.size() ) return;
  if( this->g711tol16() ) return;
  if( this->ilbctol16() ) return;

  this->g722tol16();
  this->l16widetonarrowband();
}

/*!md
Try to simplify the code. Use the operator << to take in data and take out data.

codecx << rtpacket places data into our codec.

rtppacket << codecx takes data out.

We pass a packet in, then we can take multiple out - i.e. we may want different destinations with different (or the same) CODECs.

Have a think about if this is where we want to mix audio data.
*/
codecx& operator << ( codecx& c, rtppacket& pk )
{
  rawsound r = rawsound( pk );
  c << r;
  return c;
}

codecx& operator << ( codecx& c, rawsound& raw )
{
  int inpayloadtype = raw.getformat();

  switch( inpayloadtype )
  {
    case PCMAPAYLOADTYPE:
    {
      c.pcmaref = raw;
      break;
    }
    case PCMUPAYLOADTYPE:
    {
      c.pcmuref = raw;
      break;
    }
    case ILBCPAYLOADTYPE:
    {
      c.ilbcref = raw;
      break;
    }
    case G722PAYLOADTYPE:
    {
      c.g722ref = raw;
      break;
    }
    case L16PAYLOADTYPE:
    {
      if( 8000 == raw.getsamplerate() )
      {
        c.l168kref = raw;
      }
      else if( 16000 == raw.getsamplerate() )
      {
        c.l1616kref = raw;
      }
      break;
    }
  }

  return c;
}

codecx& operator << ( codecx& c, const char& a )
{
  if( 0 == a )
  {
    c.pcmaref.size( 0 );
    c.pcmuref.size( 0 );
    c.g722ref.size( 0 );
    c.ilbcref.size( 0 );
    c.l168kref.size( 0 );
    c.l1616kref.size( 0 );
  }
  return c;
}

/*!md
## rtppacket << codecx
Take the data out and transcode if necessary. Keep reference to any packet we have transcoded as we may need to use it again.
*/
rtppacket& operator << ( rtppacket& pk, codecx& c )
{
  int outpayloadtype = pk.getpayloadtype();

  /* If we have already have or converted this packet... */
  if( PCMAPAYLOADTYPE == outpayloadtype &&  0 != c.pcmaref.size() )
  {
    pk.copy( c.pcmaref.c_str(), c.pcmaref.size() );
    return pk;
  }
  else if( PCMUPAYLOADTYPE == outpayloadtype && 0 != c.pcmuref.size() )
  {
    pk.copy( c.pcmuref.c_str(), c.pcmuref.size() );
    return pk;
  }
  else if( ILBCPAYLOADTYPE == outpayloadtype && 0 != c.ilbcref.size() )
  {
    pk.copy( c.ilbcref.c_str(), c.ilbcref.size() );
    return pk;
  }
  else if( G722PAYLOADTYPE == outpayloadtype && 0 != c.g722ref.size() )
  {
    pk.copy( c.g722ref.c_str(), c.g722ref.size() );
    return pk;
  }

  /* If we get here we may have L16 but at the wrong sample rate so check and resample - then convert */
  /* narrowband targets */
  switch( outpayloadtype )
  {
    case ILBCPAYLOADTYPE:
    {
      c.requirenarrowband();
      c.ilbcref = rawsound( pk );
      c.l16toilbc();
      pk.setpayloadlength( c.ilbcref.size() );
      break;
    }
    case G722PAYLOADTYPE:
    {
      c.requirewideband();
      c.g722ref = rawsound( pk );
      c.l16tog722();
      pk.setpayloadlength( c.g722ref.size() );
      break;
    }
    case PCMAPAYLOADTYPE:
    {
      c.pcmaref = rawsound( pk );
      if( c.pcmuref.size() > 0 )
      {
        c.alaw2ulaw();
      }
      else
      {
        c.requirenarrowband();
        c.l16topcma();
      }
      pk.setpayloadlength( c.pcmaref.size() );

      break;
    }
    case PCMUPAYLOADTYPE:
    {
      c.pcmuref = rawsound( pk );
      if( c.pcmaref.size() > 0 )
      {
        c.ulaw2alaw();
      }
      else
      {
        c.requirenarrowband();
        c.l16topcmu();
      }

      pk.setpayloadlength( c.pcmuref.size() );
      break;
    }
    default:
    {
      pk.length = 0;
    }
  }

  return pk;
}


/*!md
# rawsound
An object representing raw data for sound - which can be in any format (supported). We maintain a pointer to the raw data and will not clean it up.
*/
rawsound::rawsound() :
  data( nullptr ),
  length( 0 ),
  allocatedlength( 0 ),
  format( 0 ),
  samplerate( 0 )
{
}

/*!md
# rawsound
*/
rawsound::rawsound( uint8_t *ptr, std::size_t length, int format, uint16_t samplerate ) :
  data( ptr ),
  length( length ),
  allocatedlength( 0 ),
  format( format ),
  samplerate( samplerate )
{
}

/*!md
## rawsound
Construct from an rtp packet
*/
rawsound::rawsound( rtppacket& pk ) :
  data( pk.getpayload() ),
  length( pk.getpayloadlength() ),
  allocatedlength( 0 ),
  format( pk.getpayloadtype() )
{
  switch( pk.getpayloadtype() )
  {
    case PCMUPAYLOADTYPE:
    case PCMAPAYLOADTYPE:
    {
      this->samplerate = 8000;
      break;
    }
    case G722PAYLOADTYPE:
    {
      this->samplerate = 16000;
      break;
    }
    case ILBCPAYLOADTYPE:
    {
      this->samplerate = 8000;
      break;
    }
  }
}

/*!md
## Copy c'stor
Original maintains ownership of any allocated memory.
*/
rawsound::rawsound( rawsound &o ) :
  data( o.data ),
  length( o.length ),
  allocatedlength( 0 ),
  format( o.format ),
  samplerate( o.samplerate )
{
}

/*!md
## d-tor
Tidy up.
*/
rawsound::~rawsound()
{
  if( this->allocatedlength > 0 )
  {
    delete[] this->data;
  }
}

/*!md
## malloc
Allocate our own memory.
*/
void rawsound::malloc( size_t len )
{
  this->length = len;

  if( this->allocatedlength > 0 )
  {
    if( this->allocatedlength > len )
    {
      return;
    }

    delete[] this->data;
  }
  this->data = new uint8_t[ len ];
  this->allocatedlength = len;
}
