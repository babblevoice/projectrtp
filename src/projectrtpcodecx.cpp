
/* needed to build on ubuntu */
#include <utility>

#include <iostream>
#include <cstdlib>
#include <iomanip>
#include <math.h>

#include "projectrtpcodecx.h"
#include "projectrtprawsound.h"

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

void gen711convertdata( void ) {
  int32_t i;
  for( i = 0; i != 65536; i++ ) {
    int16_t l16val = i - 32768;
    _l16topcmu[ i ] = linear_to_ulaw( l16val );
    _l16topcma[ i ] = linear_to_alaw( l16val );
  }

  for( i = 0; i != 256; i++ ) {
    _pcmatol16[ i ] = alaw_to_linear( i );
    _pcmutol16[ i ] = ulaw_to_linear( i );
  }
}

codecx::codecx() :
  g722encoder( nullptr ),
  g722decoder( nullptr ),
  ilbcencoder( nullptr ),
  ilbcdecoder( nullptr ),
  lpfilter(),
  l168kref(),
  l1616kref(),
  pcmaref(),
  pcmuref(),
  g722ref(),
  ilbcref(),
  dcpowerfilter(),
  _hasdata( false ),
  inpkcount( 0 )
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
  this->_hasdata = false;
  this->inpkcount = 0;
}

/*!md
## restart
Do enough to manage missing packets.
*/
void codecx::restart( void ) {
  this->lpfilter.reset();
  this->dcpowerfilter.reset();
  this->_hasdata = false;
}

/*!md
## ulaw2alaw, alaw2ulaw

From whichever PCM encoding (u or a) encode to the other without having to do intermediate l16.
*/
bool codecx::alaw2ulaw( void ) {
  if( 0 == this->pcmaref.size() ) return false;
  if( this->pcmaref.isdirty() ) return false;

  uint8_t *inbufptr, *outbufptr;

  inbufptr = this->pcmaref.c_str();
  outbufptr = this->pcmuref.c_str();
  this->pcmuref.size( G711PAYLOADBYTES );

  if( nullptr == outbufptr || nullptr == inbufptr ) {
    std::cerr << "PCMA NULLPTR shouldn't happen (" << (void*)outbufptr << ", " << (void*)inbufptr << ")" << std::endl;
    return false;
  }

  for( size_t i = 0; i < G711PAYLOADBYTES; i++ ) {
    *outbufptr = alaw_to_ulaw_table[ *inbufptr ];
    inbufptr++;
    outbufptr++;
  }

  this->pcmuref.dirty( false );
  return true;
}

bool codecx::ulaw2alaw( void ) {
  if( 0 == this->pcmuref.size() ) return false;
  if( this->pcmuref.isdirty() ) return false;

  uint8_t *inbufptr, *outbufptr;

  inbufptr = this->pcmuref.c_str();
  outbufptr = this->pcmaref.c_str();
  this->pcmaref.size( G711PAYLOADBYTES );

  if( nullptr == outbufptr || nullptr == inbufptr ) {
    std::cerr << "PCMU NULLPTR shouldn't happen(" << (void*)outbufptr << ", " << (void*)inbufptr << ")" << std::endl;
    return false;
  }

  for( size_t i = 0; i < G711PAYLOADBYTES; i++ ) {
    *outbufptr = ulaw_to_alaw_table[ *inbufptr ];
    inbufptr++;
    outbufptr++;
  }

  this->pcmaref.dirty( false );
  return true;
}

/*!md
## g711tol16
As it says.
*/
bool codecx::g711tol16( void )
{
  uint8_t *in;
  int16_t *convert;

  if( this->pcmaref.size() > 0 && !this->pcmaref.isdirty() ) {
    in = this->pcmaref.c_str();
    convert = _pcmatol16;
  } else if ( this->pcmuref.size() > 0 && !this->pcmuref.isdirty() ) {
    in = this->pcmuref.c_str();
    convert = _pcmutol16;
  } else {
    return false;
  }

  this->l168kref.malloc( L16PAYLOADSAMPLES, sizeof( int16_t ), L168KPAYLOADTYPE );

  int16_t *out = ( int16_t * ) this->l168kref.c_str();

  for( size_t i = 0; i < L16PAYLOADSAMPLES; i++ ) {
    *out = convert[ *in ];
    in++;
    out++;
  }

  this->l168kref.dirty( false );
  return true;
}


/*!md
## l16topcma
*/
bool codecx::l16topcma( void )
{
  if( 0 == this->l168kref.size() ) return false;
  if( this->l168kref.isdirty() ) return false;

  uint8_t *out = this->pcmaref.c_str();

  int16_t *in;
  in = ( int16_t * ) this->l168kref.c_str();

  uint16_t index;
  for( size_t i = 0; i < G711PAYLOADBYTES; i++ ) {
    index = *in + 32768;
    *out = _l16topcma[ index ];
    in++;
    out++;
  }
  this->pcmaref.dirty( false );
  return true;
}

/*!md
## l16topcmu
*/
bool codecx::l16topcmu( void ) {
  if( 0 == this->l168kref.size() ) return false;
  if( this->l168kref.isdirty() ) return false;

  uint8_t *out = this->pcmuref.c_str();;

  int16_t *in;
  in = ( int16_t * ) this->l168kref.c_str();

  uint16_t index;
  for( size_t i = 0; i < G711PAYLOADBYTES; i++ ) {
    index = *in + 32768;
    *out = _l16topcmu[ index ];
    in++;
    out++;
  }
  this->pcmuref.dirty( false );
  return true;
}

/*!md
## ilbctol16
As it says.
*/
bool codecx::ilbctol16( void ) {
  if( 0 == this->ilbcref.size() ) return false;
  if( this->ilbcref.isdirty() ) return false;

  /* roughly compression size with some leg room. */
  this->l168kref.malloc( L16PAYLOADSAMPLES, sizeof( int16_t ), L168KPAYLOADTYPE );
  int16_t speechType;

  if( nullptr == this->ilbcdecoder ) {
    /* Only support 20mS ATM to make the mixing simpler */
    WebRtcIlbcfix_DecoderCreate( &this->ilbcdecoder );
    WebRtcIlbcfix_DecoderInit( this->ilbcdecoder, 20 );
  }

  int16_t l168klength = WebRtcIlbcfix_Decode( this->ilbcdecoder,
                        ( ilbcencodedval ) this->ilbcref.c_str(),
                        ILBC20PAYLOADBYTES,
                        ( ilbcdecodedval )this->l168kref.c_str(),
                        &speechType
                      );

  if( -1 == l168klength ) {
    return false;
  }

  this->l168kref.dirty( false );
  return true;
}

/*!md
## l16toilbc
As it says.
I think we can always send as 20mS.
According to RFC 3952 you determin how many frames are in the packet by the frame length. This is also true (although not explicit) that we can send as 20mS if our source is that but also 30mS if that is the case.

As we only support G722, G711 and iLBC (20/30) then we should be able simply encode and send as the matched size.
*/
bool codecx::l16toilbc( void ) {
  if( 0 == this->l168kref.size() ) return false;
  if( this->l168kref.isdirty() ) return false;

  if( nullptr == this->ilbcencoder ) {
    /* Only support 20mS ATM to make the mixing simpler */
    WebRtcIlbcfix_EncoderCreate( &this->ilbcencoder );
    WebRtcIlbcfix_EncoderInit( this->ilbcencoder, 20 );
  }

  int16_t len = WebRtcIlbcfix_Encode( this->ilbcencoder,
                            ( ilbcdecodedval ) this->l168kref.c_str(),
                            L16PAYLOADSAMPLES,
                            ( ilbcencodedval ) this->ilbcref.c_str()
                          );
  if ( len > 0 ) {
    this->ilbcref.dirty( false );
    return true;
  }

  this->ilbcref.size( 0 );
  return false;

}


/*!md
## l16tog722
As it says.
*/
bool codecx::l16tog722( void ) {

  if( 0 == this->l1616kref.size() ) return false;
  if( this->l1616kref.isdirty() ) return false;

  if( nullptr == this->g722encoder ) {
    this->g722encoder = g722_encode_init( NULL, 64000, G722_PACKED );
  }

  // TODO - when we convert to g722 - 722 buffer is always output -so malloc need sto be able to detect that.
  //this->g722ref.malloc( G722PAYLOADSAMPLES, sizeof( int8_t ), G722PAYLOADTYPE );

  int len = g722_encode( this->g722encoder, this->g722ref.c_str(), ( int16_t * ) this->l1616kref.c_str(), L1616PAYLOADSAMPLES );

  if( 160 != len ) {
    std::cerr << "g722_encode didn't encode correct length of data" << std::endl;
    return false;
  }

  this->g722ref.dirty( false );
  return true;
}

/*!md
## g722tol16
As it says.
*/
bool codecx::g722tol16( void ) {
  if( !this->g722ref.hasdata() ) return false;

  this->l1616kref.malloc( L1616PAYLOADSAMPLES, sizeof( int16_t ), L1616KPAYLOADTYPE );

  if( nullptr == this->g722decoder ) {
    this->g722decoder = g722_decode_init( NULL, 64000, G722_PACKED );
    if( nullptr ==  this->g722decoder ) {
      std::cerr << "Failed to init G722 decoder" << std::endl;
    }
  }

  size_t l1616klength = g722_decode( this->g722decoder,
                                ( int16_t * ) this->l1616kref.c_str(),
                                this->g722ref.c_str(),
                                G722PAYLOADBYTES );

  if( L1616PAYLOADSAMPLES != l1616klength ) return false;
  
  this->l1616kref.dirty( false );
  return true;
}

/*!md
## l16lowtowideband
Upsample from narrow to wideband. Take each point and interpolate between them. We require the final sample from the last packet to continue the interpolating.
*/
bool codecx::l16lowtowideband( void ) {

  if( 0 == this->l168kref.size() ) return false;
  if( this->l168kref.isdirty() ) return false;

  this->l1616kref.malloc( L1616PAYLOADSAMPLES, sizeof( int16_t ), L1616KPAYLOADTYPE );

  int16_t *in = ( int16_t * ) this->l168kref.c_str();
  int16_t *out = ( int16_t * ) this->l1616kref.c_str();

  for( size_t i = 0; i < L16PAYLOADSAMPLES; i++ ) {
    *out = this->lpfilter.execute( *in );
    out++;
    *out = this->lpfilter.execute( 0 );
    out++;
    in++;
  }

  this->l1616kref.dirty( false );
  return true;
}

/*!md
## requirewideband
Search for the relevent data and convert as necessary.
*/
bool codecx::requirewideband( void ) {
  if( 0 != this->l1616kref.size() && !this->l1616kref.isdirty() ) return true;

  if( 0 != this->l168kref.size() && !this->l168kref.isdirty() ) {
    return this->l16lowtowideband();
  }

  if( this->g722tol16() ) return true;
  if( !this->g711tol16() ) {
    if( !this->ilbctol16() ) {
      return false;
    }
  }

  return this->l16lowtowideband();
}

/*!md
##  l16widetolowband
Downsample our L16 wideband samples to 8K. Pass through filter then grab every other sample.
*/
bool codecx::l16widetonarrowband( void ) {

  if( !this->l1616kref.hasdata() ) return false;

  this->l168kref.malloc( L16PAYLOADSAMPLES, sizeof( int16_t ), L168KPAYLOADTYPE );

  int16_t *out = ( int16_t * ) this->l168kref.c_str();
  int16_t *in = ( int16_t * ) this->l1616kref.c_str();

  for( size_t i = 0; i < L16PAYLOADSAMPLES; i++ ) {
    this->lpfilter.execute( *in );
    in++;
    *out = this->lpfilter.execute( *in );
    in++;
    out++;
  }

  this->l168kref.dirty( false );
  return true;
}

/*!md
## requirenarrowband
Search for the relevent data and convert as necessary.
*/
bool codecx::requirenarrowband( void ) {
  if( this->l168kref.hasdata() ) return true;

  if( this->l1616kref.hasdata() ) {
    return this->l16widetonarrowband();
  }

  if( this->g711tol16() ) return true;
  if( this->ilbctol16() ) return true;
  this->g722tol16();
  return this->l16widetonarrowband();
}

/*!md
## requirel16
Wide or narrow - it doesn't matter - we just need l16
*/
rawsound& codecx::requirel16( void ) {
  if( this->l168kref.hasdata() ) return this->l168kref;
  if( this->l1616kref.hasdata() ) return this->l1616kref;

  if( this->g711tol16() ) return this->l168kref;
  if( this->ilbctol16() ) return this->l168kref;
  if( this->g722tol16() ) return this->l1616kref;

  return this->l168kref;
}

rawsound nullref;
/**
 * Obtain a reference to a rawsound for the codec type pt. i.e. we alrteady have 
 * the input sound and based on the format we want to convert it to that 
 * format.
*/
rawsound& codecx::getref( int pt ) {

  switch( pt ) {
    case PCMAPAYLOADTYPE:
      if( this->pcmaref.hasdata() ) return this->pcmaref;
      if( this->pcmuref.hasdata() ) {
        this->ulaw2alaw();
      } else {
        this->requirenarrowband();
        this->l16topcma();
      }
      return this->pcmaref;
    case PCMUPAYLOADTYPE:
      if( this->pcmuref.hasdata() ) return this->pcmuref;
      if( this->pcmaref.hasdata() ) {
        this->alaw2ulaw();
      } else {
        this->requirenarrowband();
        this->l16topcmu();
      }
      return this->pcmuref;
    case ILBCPAYLOADTYPE:
      if( this->ilbcref.hasdata() ) return this->ilbcref;
      this->requirenarrowband();
      this->l16toilbc();
      return this->ilbcref;
    case G722PAYLOADTYPE:
      if( this->g722ref.hasdata() ) return this->g722ref;
      this->requirewideband();
      this->l16tog722();

      return this->g722ref;
    case L168KPAYLOADTYPE:
      if( this->l168kref.hasdata() ) return this->l168kref;
      this->requirenarrowband();
      return this->l168kref;
    case L1616KPAYLOADTYPE:
      if( this->l1616kref.hasdata() ) return this->l1616kref;
      this->requirewideband();
      return this->l1616kref;
  }

  /* We should ever get here unless an invalid param has been passed in */
  std::cerr << "codecx::getref call with bad pt: " << pt << std::endl;
  return nullref;
}

/*
## Calculate the power in a packet
Rely on compiler to use SSE + rsqrtss for sqrt. If this ever gets ported to a different
processor with limited functions like this then fast inverse sqrt should be implemented.
*/
uint16_t codecx::power( void )
{
  if( this->inpkcount < 100 ) return 0; /* ensure the rtp has established */
  rawsound &ref = this->requirel16();
  if ( 0 == ref.size() ) return 0;

  uint32_t stotsq = 0;
  int16_t *s = ( int16_t* ) ref.c_str();
  int16_t filtered;
  for( size_t i = 0; i < ref.size(); i++ ) {
    filtered = this->dcpowerfilter.execute( *s );
    stotsq += filtered * filtered;
    s++;
  }

  stotsq = stotsq * (float)( 1 / (float) ref.size() );
  return sqrt( stotsq );
}

/*!md
Try to simplify the code. Use the operator << to take in data and take out data.

codecx << rtpacket places data into our codec.

rtppacket << codecx takes data out.

We pass a packet in, then we can take multiple out - i.e. we may want different destinations with different (or the same) CODECs.

Have a think about if this is where we want to mix audio data.
*/
codecx& operator << ( codecx& c, rtppacket& pk ) {
  c.inpkcount++;
  rawsound r = rawsound( pk );
  c << r;
  return c;
}

codecx& operator << ( codecx& c, rawsound& raw ) {
  int inpayloadtype = raw.getformat();

  switch( inpayloadtype ) {
    case PCMAPAYLOADTYPE: {
      c.pcmaref = raw;
      c._hasdata = true;
      break;
    }
    case PCMUPAYLOADTYPE: {
      c.pcmuref = raw;
      c._hasdata = true;
      break;
    }
    case ILBCPAYLOADTYPE: {
      c.ilbcref = raw;
      c._hasdata = true;
      break;
    }
    case G722PAYLOADTYPE: {
      c.g722ref = raw;
      c._hasdata = true;
      break;
    }
    case L168KPAYLOADTYPE: {
      c.l168kref = raw;
      c._hasdata = true;
      break;
    }
    case L1616KPAYLOADTYPE: {
      c.l1616kref = raw;
      c._hasdata = true;
      break;
    }
  }

  return c;
}

codecx& operator << ( codecx& c, const char& a )
{
  if( 0 == a )
  {
    c.pcmaref.dirty();
    c.pcmuref.dirty();
    c.g722ref.dirty();
    c.ilbcref.dirty();
    c.l168kref.dirty();
    c.l1616kref.dirty();
    c._hasdata = false;
  }
  return c;
}



void codectests( void ) {
  /* init transcoding stuff */
  gen711convertdata();

  /* Test a G722 packet to L16 using the underlying functions. */
  uint8_t g722samplepk[] = {
    218, 123, 22, 247, 110, 123, 54, 249, 26, 252, 115, 178, 222, 190, 217, 179, 239, 116, 238, 249,
    60, 222, 116, 249, 218, 189, 220, 121, 113, 47, 220, 92, 182, 221, 186, 223, 124, 245, 179, 218,
    53, 212, 123, 172, 181, 250, 87, 221, 215, 213, 124, 116, 126, 218, 123, 239, 252, 246, 116, 159,
    54, 122, 124, 29, 181, 215, 237, 119, 87, 121, 159, 247, 243, 252, 223, 127, 245, 255, 147, 190,
    21, 218, 247, 248, 116, 174, 88, 113, 90, 45, 243, 157, 125, 215, 109, 178, 116, 108, 213, 92, 217,
    216, 87, 120, 108, 218, 184, 185, 60, 57, 87, 244, 211, 190, 244, 53, 252, 255, 252, 156, 82, 251,
    210, 83, 217, 155, 52, 154, 88, 211, 121, 250, 94, 223, 127, 89, 29, 81, 118, 174, 90, 175, 178,
    247, 248, 113, 127, 121, 254, 215, 118, 238, 186, 119, 179, 50, 124, 251, 126, 254 };


  g722_decode_state_t *g722decoder = g722_decode_init( NULL, 64000, G722_PACKED );
  if( nullptr == g722decoder ) {
    std::cerr << "Failed to init G722 decoder" << std::endl;
  }


  int16_t outbuf[ 320 ];
  size_t l1616klength = g722_decode( g722decoder,
                                outbuf,
                                g722samplepk,
                                160 );
  std::cout << "g722_decode returned " << l1616klength << " for an in packet of 160 bytes" << std::endl;
  if( 320 != l1616klength ) {
    std::cerr << "ERROR - decoded length is not 320 bytes" << std::endl;
  }
  g722_decode_free( g722decoder );

  std::cout << "L16 OUT (g722_decode) =" << std::endl;
  for( int i = 0; i < 160; i ++ ) {
    std::cout << unsigned( outbuf[ i ] ) << " ";
  }
  std::cout << std::endl;


  /*
  Move onto test our interface.
  */
  {
    rawsound r( g722samplepk, sizeof( g722samplepk ), G722PAYLOADTYPE, 16000 );
    std::cout << "G722 raw packet size " << r.size() << " with a format of " << r.getformat() << " and sample rate " << r.getsamplerate() << std::endl;

    codecx ourcodec;

    ourcodec << codecx::next;
    ourcodec << r;

    

    rtppacket outpk;
    outpk.setpayloadtype( PCMAPAYLOADTYPE );
    outpk.setpayloadlength( 160 );
    outpk.setsequencenumber( 0 );
    outpk.settimestamp( 0 );
    outpk << ourcodec;

    std::cout << "PCMA OUT =" << std::endl;
    uint8_t *pl = outpk.getpayload();

    for( int i = 0; i < 160; i ++ ) {
      std::cout << unsigned( pl[ i ] ) << " ";
    }
    std::cout << std::endl;

    if( 213 != pl[ 0 ] ) std::cout << "ERROR ERROR First byte should be 213???" << std::endl;
    if( 85 != pl[ 9 ] ) std::cout << "ERROR ERROR 9th byte should be 85???" << std::endl;

    rawsound &ref8k = ourcodec.getref( L168KPAYLOADTYPE );

    if( ref8k.isdirty() ) std::cout << "ERROR our 8k out ref should not be dirty" << std::endl;

    /* Repeat as this will use a different bit of code getting cached bit */
    ref8k = ourcodec.getref( L168KPAYLOADTYPE );
    if( ref8k.isdirty() ) std::cout << "ERROR our 8k out ref should not be dirty" << std::endl;

    std::cout << "8k is dirty: " << std::boolalpha << ref8k.isdirty() << std::endl;
    std::cout << "8k size: " << ref8k.size() << std::endl;
    std::cout << "8k bytes per sample (should be 2): " << ref8k.getbytespersample() << std::endl;

    /* 2 bytes per sample */
    int16_t *pl16 = ( int16_t * ) ref8k.c_str();
    for( size_t i = 0; i < ref8k.size(); i ++ ) {
      std::cout << static_cast<int16_t>( pl16[ i ] ) << " ";
    }
    std::cout << std::endl;


    rawsound &ref16k = ourcodec.getref( L1616KPAYLOADTYPE );
    if( ref16k.isdirty() ) std::cout << "ERROR our 16k out ref should not be dirty" << std::endl;
    std::cout << "16k is dirty: " << std::boolalpha << ref16k.isdirty() << std::endl;
    std::cout << "16k size: " << ref16k.size() << std::endl;
    std::cout << "16k bytes per sample (should be 2): " << ref16k.getbytespersample() << std::endl;

    pl16 = ( int16_t * ) ref16k.c_str();
    for( size_t i = 0; i < ref16k.size(); i ++ ) {
      std::cout << static_cast<int16_t>( pl16[ i ] ) << " ";
    }
    std::cout << std::endl;
  }


  {
    /* problem with converting pcmu to pcma */
    /* use the same data, but pretend it is PCMU */
    rawsound r( g722samplepk, sizeof( g722samplepk ), PCMUPAYLOADTYPE, 8000 );

    codecx ourcodec;

    ourcodec << codecx::next;
    ourcodec << r;

    rtppacket outpk;
    outpk.setpayloadtype( PCMAPAYLOADTYPE );
    outpk.setpayloadlength( 160 );
    outpk.setsequencenumber( 0 );
    outpk.settimestamp( 0 );
    outpk << ourcodec;

    std::cout << "PCMA OUT (it should not be all zeros) = " << std::endl;

    uint8_t *pl = outpk.getpayload();

    for( int i = 0; i < 160; i ++ ) {
      std::cout << unsigned( pl[ i ] ) << " ";
    }
    std::cout << std::endl;
  }

    {
    /* problem with converting pcmu to pcma */
    /* use the same data, but pretend it is PCMU */
    rawsound r( g722samplepk, sizeof( g722samplepk ), PCMAPAYLOADTYPE, 8000 );

    codecx ourcodec;

    ourcodec << codecx::next;
    ourcodec << r;

    rtppacket outpk;
    outpk.setpayloadtype( PCMUPAYLOADTYPE );
    outpk.setpayloadlength( 160 );
    outpk.setsequencenumber( 0 );
    outpk.settimestamp( 0 );
    outpk << ourcodec;

    std::cout << "PCMU OUT (it should not be all zeros) = " << std::endl;

    uint8_t *pl = outpk.getpayload();

    for( int i = 0; i < 160; i ++ ) {
      std::cout << unsigned( pl[ i ] ) << " ";
    }
    std::cout << std::endl;
  }
  
}

#ifdef NODE_MODULE

static napi_value codectest( napi_env env, napi_callback_info info ) {
  napi_value result;
  if( napi_ok != napi_create_object( env, &result ) ) return NULL;

  codectests();

  napi_create_uint32( env, 1, &result );
  napi_coerce_to_bool( env, result, &result );

  return result;
}

/*
Support single number just for now - but TODO detect Buffer input to convert whole bufffer.
*/
static napi_value linear2pcma( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  int32_t inval;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_type_error( env, "0", "Data required" );
    return NULL;
  }

  if( napi_ok != napi_get_value_int32( env, argv[ 0 ], &inval ) ) return NULL;
  inval = inval + 32768 /* ( 2^16 ) / 2 */;

  if ( inval > 65536 /* ( 2^16 ) */ ) return NULL;
  if ( inval < 0 ) return NULL;

  napi_value returnval = NULL;
  if( napi_ok != napi_create_int32( env, _l16topcma[ inval ], &returnval ) ) return NULL;
  return returnval;
}

static napi_value pcma2linear( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  int32_t inval;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_type_error( env, "0", "Data required" );
    return NULL;
  }

  if( napi_ok != napi_get_value_int32( env, argv[ 0 ], &inval ) ) return NULL;
  if ( inval > 256 /* 2 ^ 8 */ ) return NULL;
  if ( inval < 0 ) return NULL;

  napi_value returnval = NULL;
  if( napi_ok != napi_create_int32( env, _pcmatol16[ inval ], &returnval ) ) return NULL;
  return returnval;
}

static napi_value linear2pcmu( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  int32_t inval;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_type_error( env, "0", "Data required" );
    return NULL;
  }

  if( napi_ok != napi_get_value_int32( env, argv[ 0 ], &inval ) ) return NULL;
  inval = inval + 32768 /* ( 2^16 ) / 2 */;

  if ( inval > 65536 /* ( 2^16 ) */ ) return NULL;
  if ( inval < 0 ) return NULL;

  napi_value returnval = NULL;
  if( napi_ok != napi_create_int32( env, _l16topcmu[ inval ], &returnval ) ) return NULL;
  return returnval;
}

static napi_value pcmu2linear( napi_env env, napi_callback_info info ) {
  size_t argc = 1;
  napi_value argv[ 1 ];
  int32_t inval;

  if( napi_ok != napi_get_cb_info( env, info, &argc, argv, nullptr, nullptr ) ) return NULL;

  if( 1 != argc ) {
    napi_throw_type_error( env, "0", "Data required" );
    return NULL;
  }

  if( napi_ok != napi_get_value_int32( env, argv[ 0 ], &inval ) ) return NULL;
  if ( inval > 256 /* 2 ^ 8 */ ) return NULL;
  if ( inval < 0 ) return NULL;

  napi_value returnval = NULL;
  if( napi_ok != napi_create_int32( env, _pcmutol16[ inval ], &returnval ) ) return NULL;
  return returnval;
}

void initrtpcodecx( napi_env env, napi_value &result ) {
  napi_value codecx;
  napi_value funct;

  if( napi_ok != napi_create_object( env, &codecx ) ) return;
  if( napi_ok != napi_set_named_property( env, result, "codecx", codecx ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, linear2pcma, nullptr, &funct ) ) return;
  if( napi_ok != napi_set_named_property( env, codecx, "linear162pcma", funct ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, pcma2linear, nullptr, &funct ) ) return;
  if( napi_ok != napi_set_named_property( env, codecx, "pcma2linear16", funct ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, linear2pcmu, nullptr, &funct ) ) return;
  if( napi_ok != napi_set_named_property( env, codecx, "linear162pcmu", funct ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, pcmu2linear, nullptr, &funct ) ) return;
  if( napi_ok != napi_set_named_property( env, codecx, "pcmu2linear16", funct ) ) return;

  if( napi_ok != napi_create_function( env, "exports", NAPI_AUTO_LENGTH, codectest, nullptr, &funct ) ) return;
  if( napi_ok != napi_set_named_property( env, codecx, "codectests", funct ) ) return;
}

#endif /* NODE_MODULE */
