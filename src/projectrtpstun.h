
#ifndef PROJECTRTPSTUN_H
#define PROJECTRTPSTUN_H

/* boost endpoint */
#include <boost/asio.hpp>

#define STUNMESSAGETYPE_BINDINGREQUEST 0x01
#define STUNMESSAGETYPE_BINDINGRESPONSE 0x11

class stun: 
  public std::enable_shared_from_this< stun > {

public:

  stun();

  /* parse */
  static bool is( uint8_t *pk, size_t len );
  static size_t handle( uint8_t *pk, uint8_t * response, size_t responselength, boost::asio::ip::udp::endpoint &endpoint, std::string &localkey, std::string &remotekey );

};

#endif /* PROJECTRTPSTUN_H */
