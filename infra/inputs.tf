variable "region" {
  type        = string
  description = "The region that AWS will default to"
  nullable    = false
}

variable "domain" {
  type        = string
  description = "The domain to host on"
  nullable    = false
}

variable "base_domain" {
  type        = string
  description = "If domain is a subdomain, then this needs to be the root domain in order to find the hosted zone"
  default     = null
}

variable "name" {
  type        = string
  description = "The name of the deployment"
  default     = "url_short"
}

variable "default_redirect" {
  type        = string
  description = "The default redirect to use if no key is provided"
  nullable    = false
}

variable "admin_key" {
  type        = string
  description = "The key to access the admin panel"
  nullable    = false
}

variable "admin_secret" {
  type        = string
  description = "The secret to access the admin panel"
  nullable    = false
}
